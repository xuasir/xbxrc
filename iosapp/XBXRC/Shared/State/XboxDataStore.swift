import Combine
import Foundation

enum HostPowerCommand: String, Equatable, Sendable {
    case powerOn
    case powerOff

    fileprivate var failureMessage: String {
        switch self {
        case .powerOn: "无法开启主机，请稍后重试"
        case .powerOff: "无法关闭主机，请稍后重试"
        }
    }
}

enum HostPowerCommandState: Equatable, Sendable {
    case idle
    case executing(hostID: String, command: HostPowerCommand)
    case failed(hostID: String, command: HostPowerCommand, message: String)

    var isExecuting: Bool {
        if case .executing = self { true } else { false }
    }

    var hostID: String? {
        switch self {
        case .idle: nil
        case let .executing(hostID, _), let .failed(hostID, _, _): hostID
        }
    }

    var command: HostPowerCommand? {
        switch self {
        case .idle: nil
        case let .executing(_, command), let .failed(_, command, _): command
        }
    }

    var errorMessage: String? {
        if case let .failed(_, _, message) = self { message } else { nil }
    }
}

enum XboxDataRefreshReason: String, Equatable, Sendable {
    case initialActivation
    case manualPull
    case manualRetry
    case commandResult
}

private enum XboxDataSurface: Hashable {
    case hosts
    case library
    case achievements(String)
}

@MainActor
final class XboxDataStore: ObservableObject {
    @Published private(set) var hosts: [XboxHostSummary] = []
    @Published private(set) var hostPhase: DataLoadPhase = .idle
    @Published private(set) var isRefreshingHosts = false
    @Published private(set) var hostErrorMessage: String?
    @Published private(set) var hostPowerCommandState: HostPowerCommandState = .idle
    @Published private(set) var games: [GameSummary] = []
    @Published private(set) var libraryPhase: DataLoadPhase = .idle
    @Published private(set) var isRefreshingLibrary = false
    @Published private(set) var libraryErrorMessage: String?

    @Published private var achievementsByTitleID: [String: [AchievementSummary]] = [:]
    @Published private var achievementPhases: [String: DataLoadPhase] = [:]
    @Published private var achievementErrors: [String: String] = [:]

    private let client: any XboxDataClient
    private let preferredGameLocaleProvider: (any PreferredGameLocaleProviding)?
    private var webTokenJSON: String?
    private var boundOwnerGeneration: UInt64?
    private var fallbackOwnerKey: String?
    private var initialActivations: Set<XboxDataSurface> = []
    private var hostRefreshTask: Task<Void, Never>?
    private var hostRefreshTaskID: UUID?
    private var libraryRefreshTask: Task<Void, Never>?
    private var libraryRefreshTaskID: UUID?
    private var achievementRefreshTasks: [String: Task<Void, Never>] = [:]
    private var achievementRefreshTaskIDs: [String: UUID] = [:]
    private var hostRefreshOperationID: String?
    private var hostPowerOperationID: String?

    init(
        client: any XboxDataClient = RustXboxDataClient(),
        preferredGameLocaleProvider: (any PreferredGameLocaleProviding)? = nil
    ) {
        self.client = client
        self.preferredGameLocaleProvider = preferredGameLocaleProvider
    }

    func sync(
        session: StoredAuthSession?,
        ownerGeneration: UInt64? = nil
    ) async {
        guard let session else {
            IOSRuntimeTrace.state(
                domain: "xbox-data",
                event: "dataStoreCleared",
                payload: ["reason": "signedOut"],
                dimension: .lifecycle,
                importance: .key
            )
            clear()
            return
        }
        let nextFallbackOwnerKey = session.cloudAccountID ?? session.seedJSON
        let ownerChanged: Bool
        if let ownerGeneration {
            ownerChanged = boundOwnerGeneration != ownerGeneration
        } else {
            ownerChanged = fallbackOwnerKey != nextFallbackOwnerKey
        }
        guard ownerChanged || session.webTokenJSON != webTokenJSON else {
            IOSRuntimeTrace.decision(
                domain: "xbox-data",
                event: "dataSyncSkipped",
                payload: ["reason": "sessionUnchanged"],
                dimension: .lifecycle,
                importance: .debug
            )
            return
        }

        IOSRuntimeTrace.event(
            domain: "xbox-data",
            event: "dataSessionBound",
            payload: [
                "ownerChanged": .bool(ownerChanged),
                "previousGames": .integer(Int64(games.count)),
            ],
            dimension: .lifecycle,
            importance: .key
        )
        if ownerChanged {
            clearContent()
        }
        boundOwnerGeneration = ownerGeneration
        fallbackOwnerKey = nextFallbackOwnerKey
        webTokenJSON = session.webTokenJSON
    }

    func activateHostsOnce() async {
        await activateOnce(.hosts) {
            await self.refreshHosts(reason: .initialActivation)
        }
    }

    func activateLibraryOnce() async {
        await activateOnce(.library) {
            await self.refreshLibrary(reason: .initialActivation)
        }
    }

    func refreshHosts(
        reason: XboxDataRefreshReason = .manualPull
    ) async {
        if let hostRefreshTask {
            IOSRuntimeTrace.decision(
                domain: "xbox-data",
                event: "hostsRefreshCoalesced",
                payload: ["reason": .string(reason.rawValue)],
                dimension: .lifecycle,
                importance: .debug,
                operationID: hostRefreshOperationID
            )
            await hostRefreshTask.value
            return
        }
        let taskID = UUID()
        let task = Task { @MainActor [weak self] in
            guard let self else { return }
            await self.performRefreshHosts(reason: reason)
        }
        hostRefreshTask = task
        hostRefreshTaskID = taskID
        await task.value
        if hostRefreshTaskID == taskID {
            hostRefreshTask = nil
            hostRefreshTaskID = nil
        }
    }

    private func performRefreshHosts(reason: XboxDataRefreshReason) async {
        guard let webTokenJSON else {
            clear()
            return
        }
        guard hostRefreshOperationID == nil else {
            IOSRuntimeTrace.decision(
                domain: "xbox-data",
                event: "hostsRefreshSkipped",
                payload: ["reason": "requestInFlight"],
                dimension: .network,
                importance: .debug
            )
            return
        }
        let operationID = UUID().uuidString
        hostRefreshOperationID = operationID
        defer {
            if hostRefreshOperationID == operationID {
                hostRefreshOperationID = nil
            }
        }
        let startedAt = Date()
        let requestToken = webTokenJSON
        let requestGeneration = boundOwnerGeneration
        let requestFallbackOwnerKey = fallbackOwnerKey
        let hadContent = !hosts.isEmpty
        if hadContent {
            isRefreshingHosts = true
        } else {
            hostPhase = .loading
        }
        hostErrorMessage = nil
        IOSRuntimeTrace.event(
            domain: "xbox-data",
            event: "hostsRefreshStarted",
            payload: [
                "hadContent": .bool(hadContent),
                "existingHosts": .integer(Int64(hosts.count)),
                "reason": .string(reason.rawValue),
            ],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
        do {
            let loadedHosts = try await client.loadHosts(webTokenJSON: requestToken)
            guard ownsRequest(
                generation: requestGeneration,
                fallbackOwnerKey: requestFallbackOwnerKey
            ) else { return }
            hosts = loadedHosts
            hostPhase = .loaded
            isRefreshingHosts = false
            IOSRuntimeTrace.state(
                domain: "xbox-data",
                event: "hostsRefreshSucceeded",
                payload: [
                    "hosts": .integer(Int64(loadedHosts.count)),
                    "elapsedMs": .integer(
                        Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))
                    ),
                ],
                dimension: .core,
                importance: .key,
                operationID: operationID
            )
        } catch {
            guard ownsRequest(
                generation: requestGeneration,
                fallbackOwnerKey: requestFallbackOwnerKey
            ) else { return }
            isRefreshingHosts = false
            hostErrorMessage = "无法载入主机，请稍后重试"
            hostPhase = hosts.isEmpty ? .failed : .loaded
            IOSRuntimeTrace.event(
                domain: "xbox-data",
                event: "hostsRefreshFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "hadContent": .bool(hadContent),
                        "elapsedMs": .integer(
                            Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))
                        ),
                    ]
                ),
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
        }
    }

    func powerOn(host: XboxHostSummary) async {
        await runPowerCommand(.powerOn, host: host)
    }

    func powerOff(host: XboxHostSummary) async {
        await runPowerCommand(.powerOff, host: host)
    }

    func refreshLibrary(
        reason: XboxDataRefreshReason = .manualPull
    ) async {
        if let libraryRefreshTask {
            IOSRuntimeTrace.decision(
                domain: "xbox-data",
                event: "libraryRefreshCoalesced",
                payload: ["reason": .string(reason.rawValue)],
                dimension: .lifecycle,
                importance: .debug
            )
            await libraryRefreshTask.value
            return
        }
        let taskID = UUID()
        let task = Task { @MainActor [weak self] in
            guard let self else { return }
            await self.performRefreshLibrary(reason: reason)
        }
        libraryRefreshTask = task
        libraryRefreshTaskID = taskID
        await task.value
        if libraryRefreshTaskID == taskID {
            libraryRefreshTask = nil
            libraryRefreshTaskID = nil
        }
    }

    private func performRefreshLibrary(reason: XboxDataRefreshReason) async {
        guard let webTokenJSON else {
            IOSRuntimeTrace.decision(
                domain: "xbox-data",
                event: "libraryRefreshSkipped",
                payload: ["reason": "missingSession"],
                dimension: .lifecycle,
                importance: .key
            )
            clear()
            return
        }
        let operationID = UUID().uuidString
        let startedAt = Date()
        let requestToken = webTokenJSON
        let requestGeneration = boundOwnerGeneration
        let requestFallbackOwnerKey = fallbackOwnerKey
        let hadContent = !games.isEmpty
        if hadContent {
            isRefreshingLibrary = true
        } else {
            libraryPhase = .loading
        }
        libraryErrorMessage = nil
        IOSRuntimeTrace.event(
            domain: "xbox-data",
            event: "libraryRefreshStarted",
            payload: [
                "hadContent": .bool(hadContent),
                "existingGames": .integer(Int64(games.count)),
                "reason": .string(reason.rawValue),
            ],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )

        let loadedGames: [GameSummary]
        do {
            loadedGames = try await client.loadGameLibrary(webTokenJSON: requestToken)
            guard ownsRequest(
                generation: requestGeneration,
                fallbackOwnerKey: requestFallbackOwnerKey
            ) else {
                IOSRuntimeTrace.decision(
                    domain: "xbox-data",
                    event: "libraryRefreshDiscarded",
                    payload: ["stage": "library"],
                    dimension: .lifecycle,
                    importance: .key,
                    operationID: operationID
                )
                return
            }
            games = loadedGames
            libraryPhase = .loaded
            isRefreshingLibrary = true
            IOSRuntimeTrace.state(
                domain: "xbox-data",
                event: "libraryContentCommitted",
                payload: [
                    "games": .integer(Int64(loadedGames.count)),
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .core,
                importance: .key,
                operationID: operationID
            )

        } catch {
            guard ownsRequest(
                generation: requestGeneration,
                fallbackOwnerKey: requestFallbackOwnerKey
            ) else {
                IOSRuntimeTrace.decision(
                    domain: "xbox-data",
                    event: "libraryErrorDiscarded",
                    payload: ["stage": "library"],
                    dimension: .lifecycle,
                    importance: .debug,
                    operationID: operationID
                )
                return
            }
            isRefreshingLibrary = false
            libraryErrorMessage = "无法载入游戏，请稍后重试"
            libraryPhase = games.isEmpty ? .failed : .loaded
            IOSRuntimeTrace.event(
                domain: "xbox-data",
                event: "libraryRefreshFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "hadContent": .bool(hadContent),
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ]
                ),
                dimension: .network,
                importance: .essential,
                operationID: operationID
            )
            return
        }

        do {
            let playtimes = try await client.loadPlaytimes(
                webTokenJSON: requestToken,
                titleIDs: Array(loadedGames.prefix(100).map(\.titleID))
            )
            guard ownsRequest(
                generation: requestGeneration,
                fallbackOwnerKey: requestFallbackOwnerKey
            ) else {
                IOSRuntimeTrace.decision(
                    domain: "xbox-data",
                    event: "libraryRefreshDiscarded",
                    payload: ["stage": "playtimes"],
                    dimension: .lifecycle,
                    importance: .key,
                    operationID: operationID
                )
                return
            }
            merge(playtimes: playtimes)
            isRefreshingLibrary = false
            IOSRuntimeTrace.state(
                domain: "xbox-data",
                event: "libraryRefreshSucceeded",
                payload: [
                    "games": .integer(Int64(games.count)),
                    "playtimes": .integer(Int64(playtimes.count)),
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .core,
                importance: .key,
                operationID: operationID
            )
        } catch {
            guard ownsRequest(
                generation: requestGeneration,
                fallbackOwnerKey: requestFallbackOwnerKey
            ) else {
                IOSRuntimeTrace.decision(
                    domain: "xbox-data",
                    event: "libraryErrorDiscarded",
                    payload: ["stage": "playtimes"],
                    dimension: .lifecycle,
                    importance: .debug,
                    operationID: operationID
                )
                return
            }
            isRefreshingLibrary = false
            libraryErrorMessage = "游戏已载入，游玩时长暂时无法更新"
            libraryPhase = .loaded
            IOSRuntimeTrace.event(
                domain: "xbox-data",
                event: "playtimeRefreshFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "games": .integer(Int64(games.count)),
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ]
                ),
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
        }
    }

    func achievements(for titleID: String) -> [AchievementSummary] {
        achievementsByTitleID[titleID] ?? []
    }

    func achievementPhase(for titleID: String) -> DataLoadPhase {
        achievementPhases[titleID] ?? .idle
    }

    func achievementError(for titleID: String) -> String? {
        achievementErrors[titleID]
    }

    func activateAchievementsOnce(for game: GameSummary) async {
        await activateOnce(.achievements(game.titleID)) {
            await self.loadAchievements(
                for: game,
                force: false,
                reason: .initialActivation
            )
        }
    }

    func refreshAchievements(for game: GameSummary) async {
        await loadAchievements(for: game, force: true, reason: .manualPull)
    }

    func loadAchievements(
        for game: GameSummary,
        force: Bool = false,
        reason: XboxDataRefreshReason? = nil
    ) async {
        let refreshReason = reason ?? (force ? .manualPull : .initialActivation)
        if let achievementRefreshTask = achievementRefreshTasks[game.titleID] {
            IOSRuntimeTrace.decision(
                domain: "xbox-data",
                event: "achievementsRefreshCoalesced",
                payload: [
                    "titleID": .string(game.titleID),
                    "reason": .string(refreshReason.rawValue),
                ],
                dimension: .lifecycle,
                importance: .debug
            )
            await achievementRefreshTask.value
            return
        }
        if !force, achievementPhases[game.titleID] == .loaded {
            IOSRuntimeTrace.decision(
                domain: "xbox-data",
                event: "achievementsLoadSkipped",
                payload: ["reason": "alreadyLoaded"],
                dimension: .core,
                importance: .debug
            )
            return
        }

        let taskID = UUID()
        let task = Task { @MainActor [weak self] in
            guard let self else { return }
            await self.performLoadAchievements(for: game, reason: refreshReason)
        }
        achievementRefreshTasks[game.titleID] = task
        achievementRefreshTaskIDs[game.titleID] = taskID
        await task.value
        clearAchievementTask(for: game.titleID, matching: taskID)
    }

    private func performLoadAchievements(
        for game: GameSummary,
        reason: XboxDataRefreshReason
    ) async {
        guard let webTokenJSON else {
            IOSRuntimeTrace.decision(
                domain: "xbox-data",
                event: "achievementsLoadSkipped",
                payload: ["reason": "missingSession"],
                dimension: .lifecycle,
                importance: .debug
            )
            return
        }

        let operationID = UUID().uuidString
        let startedAt = Date()
        let requestToken = webTokenJSON
        let requestLocale = resolvedPreferredGameLocale()
        let requestGeneration = boundOwnerGeneration
        let requestFallbackOwnerKey = fallbackOwnerKey
        let hadContent = !(achievementsByTitleID[game.titleID] ?? []).isEmpty
        if !hadContent {
            achievementPhases[game.titleID] = .loading
        }
        achievementErrors[game.titleID] = nil
        IOSRuntimeTrace.event(
            domain: "xbox-data",
            event: "achievementsLoadStarted",
            payload: [
                "hadContent": .bool(hadContent),
                "reason": .string(reason.rawValue),
            ],
            dimension: .network,
            importance: .debug,
            operationID: operationID
        )
        do {
            let achievements = try await client.loadAchievements(
                webTokenJSON: requestToken,
                titleID: game.titleID,
                locale: requestLocale
            )
            guard ownsRequest(
                generation: requestGeneration,
                fallbackOwnerKey: requestFallbackOwnerKey
            ) else {
                IOSRuntimeTrace.decision(
                    domain: "xbox-data",
                    event: "achievementsLoadDiscarded",
                    payload: [:],
                    dimension: .lifecycle,
                    importance: .debug,
                    operationID: operationID
                )
                return
            }
            achievementsByTitleID[game.titleID] = achievements
            achievementPhases[game.titleID] = .loaded
            IOSRuntimeTrace.state(
                domain: "xbox-data",
                event: "achievementsLoadSucceeded",
                payload: [
                    "achievements": .integer(Int64(achievements.count)),
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .core,
                importance: .debug,
                operationID: operationID
            )
        } catch {
            guard ownsRequest(
                generation: requestGeneration,
                fallbackOwnerKey: requestFallbackOwnerKey
            ) else {
                return
            }
            achievementPhases[game.titleID] = hadContent ? .loaded : .failed
            achievementErrors[game.titleID] = error.localizedDescription
            IOSRuntimeTrace.event(
                domain: "xbox-data",
                event: "achievementsLoadFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ]
                ),
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
        }
    }

    private func merge(playtimes: [TitlePlaytime]) {
        let values = Dictionary(uniqueKeysWithValues: playtimes.map { ($0.titleID, $0.minutes) })
        games = games.map { game in
            GameSummary(
                id: game.id,
                titleID: game.titleID,
                name: game.name,
                artworkURL: game.artworkURL,
                heroURL: game.heroURL,
                lastPlayedAt: game.lastPlayedAt,
                playtimeMinutes: values[game.titleID] ?? game.playtimeMinutes,
                achievementProgress: game.achievementProgress
            )
        }
    }

    private func resolvedPreferredGameLocale() -> String {
        let trimmed = preferredGameLocaleProvider?.preferredGameLocale
            .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return trimmed.isEmpty ? "en-US" : trimmed
    }

    private func runPowerCommand(
        _ command: HostPowerCommand,
        host: XboxHostSummary
    ) async {
        guard hostPowerOperationID == nil else {
            IOSRuntimeTrace.decision(
                domain: "xbox-data",
                event: "hostPowerCommandSkipped",
                payload: [
                    "command": .string(command.rawValue),
                    "reason": "requestInFlight",
                ],
                dimension: .network,
                importance: .debug
            )
            return
        }
        guard let webTokenJSON else {
            hostPowerCommandState = .failed(
                hostID: host.id,
                command: command,
                message: "登录状态已失效，请重新登录"
            )
            return
        }
        guard let commandID = host.commandID?
            .trimmingCharacters(in: .whitespacesAndNewlines),
            !commandID.isEmpty
        else {
            hostPowerCommandState = .failed(
                hostID: host.id,
                command: command,
                message: "主机缺少远程控制标识"
            )
            return
        }

        let operationID = UUID().uuidString
        let startedAt = Date()
        let requestToken = webTokenJSON
        let requestGeneration = boundOwnerGeneration
        let requestFallbackOwnerKey = fallbackOwnerKey
        hostPowerOperationID = operationID
        hostPowerCommandState = .executing(hostID: host.id, command: command)
        defer {
            if hostPowerOperationID == operationID {
                hostPowerOperationID = nil
            }
        }
        IOSRuntimeTrace.event(
            domain: "xbox-data",
            event: "hostPowerCommandStarted",
            payload: ["command": .string(command.rawValue)],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )

        do {
            let result: HostPowerCommandResult
            switch command {
            case .powerOn:
                result = try await client.powerOn(
                    webTokenJSON: requestToken,
                    consoleID: commandID
                )
            case .powerOff:
                result = try await client.powerOff(
                    webTokenJSON: requestToken,
                    consoleID: commandID
                )
            }
            guard ownsRequest(
                generation: requestGeneration,
                fallbackOwnerKey: requestFallbackOwnerKey
            ),
                  hostPowerOperationID == operationID
            else { return }
            guard result.accepted else {
                throw HostPowerCommandFailure.rejected
            }

            IOSRuntimeTrace.event(
                domain: "xbox-data",
                event: "hostPowerCommandSucceeded",
                payload: [
                    "command": .string(command.rawValue),
                    "elapsedMs": .integer(
                        Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))
                    ),
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            await refreshHosts(reason: .commandResult)
            guard ownsRequest(
                generation: requestGeneration,
                fallbackOwnerKey: requestFallbackOwnerKey
            ),
                  hostPowerOperationID == operationID
            else { return }
            hostPowerCommandState = .idle
        } catch {
            guard ownsRequest(
                generation: requestGeneration,
                fallbackOwnerKey: requestFallbackOwnerKey
            ),
                  hostPowerOperationID == operationID
            else { return }
            hostPowerCommandState = .failed(
                hostID: host.id,
                command: command,
                message: command.failureMessage
            )
            IOSRuntimeTrace.event(
                domain: "xbox-data",
                event: "hostPowerCommandFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "command": .string(command.rawValue),
                        "elapsedMs": .integer(
                            Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))
                        ),
                    ]
                ),
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
        }
    }

    private func clear() {
        webTokenJSON = nil
        boundOwnerGeneration = nil
        fallbackOwnerKey = nil
        clearContent()
    }

    private func clearContent() {
        hostRefreshTask?.cancel()
        hostRefreshTask = nil
        hostRefreshTaskID = nil
        libraryRefreshTask?.cancel()
        libraryRefreshTask = nil
        libraryRefreshTaskID = nil
        achievementRefreshTasks.values.forEach { $0.cancel() }
        achievementRefreshTasks = [:]
        achievementRefreshTaskIDs = [:]
        initialActivations = []
        hostRefreshOperationID = nil
        hostPowerOperationID = nil
        hosts = []
        hostPhase = .idle
        isRefreshingHosts = false
        hostErrorMessage = nil
        hostPowerCommandState = .idle
        games = []
        libraryPhase = .idle
        isRefreshingLibrary = false
        libraryErrorMessage = nil
        achievementsByTitleID = [:]
        achievementPhases = [:]
        achievementErrors = [:]
    }

    private func activateOnce(
        _ surface: XboxDataSurface,
        operation: () async -> Void
    ) async {
        guard webTokenJSON != nil else {
            return
        }
        guard initialActivations.insert(surface).inserted else {
            IOSRuntimeTrace.decision(
                domain: "xbox-data",
                event: "surfaceActivationSkipped",
                payload: ["reason": "alreadyActivated"],
                dimension: .lifecycle,
                importance: .debug
            )
            return
        }
        await operation()
    }

    private func ownsRequest(
        generation: UInt64?,
        fallbackOwnerKey requestFallbackOwnerKey: String?
    ) -> Bool {
        if let generation {
            return boundOwnerGeneration == generation
        }
        return fallbackOwnerKey == requestFallbackOwnerKey
    }

    private func clearAchievementTask(for titleID: String, matching taskID: UUID) {
        guard achievementRefreshTaskIDs[titleID] == taskID else { return }
        achievementRefreshTasks[titleID] = nil
        achievementRefreshTaskIDs[titleID] = nil
    }
}

private enum HostPowerCommandFailure: Error {
    case rejected
}
