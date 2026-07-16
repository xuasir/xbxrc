import Combine
import Foundation

@MainActor
final class XboxDataStore: ObservableObject {
    @Published private(set) var games: [GameSummary] = []
    @Published private(set) var libraryPhase: DataLoadPhase = .idle
    @Published private(set) var isRefreshingLibrary = false
    @Published private(set) var libraryErrorMessage: String?

    @Published private var achievementsByTitleID: [String: [AchievementSummary]] = [:]
    @Published private var achievementPhases: [String: DataLoadPhase] = [:]
    @Published private var achievementErrors: [String: String] = [:]

    private let client: any XboxDataClient
    private var webTokenJSON: String?

    init(client: any XboxDataClient = RustXboxDataClient()) {
        self.client = client
    }

    func sync(session: StoredAuthSession?) async {
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
        guard session.webTokenJSON != webTokenJSON else {
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
            event: "dataSyncStarted",
            payload: ["previousGames": .integer(Int64(games.count))],
            dimension: .lifecycle,
            importance: .key
        )
        clearContent()
        webTokenJSON = session.webTokenJSON
        await refreshLibrary()
    }

    func refreshLibrary() async {
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
            ],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )

        let loadedGames: [GameSummary]
        do {
            loadedGames = try await client.loadGameLibrary(webTokenJSON: requestToken)
            guard requestToken == self.webTokenJSON else {
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
            guard requestToken == self.webTokenJSON else {
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
            guard requestToken == self.webTokenJSON else {
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
            guard requestToken == self.webTokenJSON else {
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

    func loadAchievements(for game: GameSummary, force: Bool = false) async {
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

        let operationID = UUID().uuidString
        let startedAt = Date()
        let requestToken = webTokenJSON
        achievementPhases[game.titleID] = .loading
        achievementErrors[game.titleID] = nil
        IOSRuntimeTrace.event(
            domain: "xbox-data",
            event: "achievementsLoadStarted",
            payload: ["force": .bool(force)],
            dimension: .network,
            importance: .debug,
            operationID: operationID
        )
        do {
            let achievements = try await client.loadAchievements(
                webTokenJSON: requestToken,
                titleID: game.titleID
            )
            guard requestToken == self.webTokenJSON else {
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
            guard requestToken == self.webTokenJSON else {
                return
            }
            achievementPhases[game.titleID] = .failed
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

    private func clear() {
        webTokenJSON = nil
        clearContent()
    }

    private func clearContent() {
        games = []
        libraryPhase = .idle
        isRefreshingLibrary = false
        libraryErrorMessage = nil
        achievementsByTitleID = [:]
        achievementPhases = [:]
        achievementErrors = [:]
    }
}
