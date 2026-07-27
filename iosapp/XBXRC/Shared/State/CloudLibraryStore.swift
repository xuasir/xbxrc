import Combine
import Foundation

enum CloudLibraryDiagnostics {
    static func elapsedMilliseconds(since start: Date) -> Int {
        max(0, Int(Date().timeIntervalSince(start) * 1_000))
    }

    static func errorPayload(
        _ error: any Error,
        extra: [String: IOSRuntimeTraceValue] = [:]
    ) -> [String: IOSRuntimeTraceValue] {
        let nsError = error as NSError
        let safeMessage = safeError(error)
        var payload = extra
        payload["errorDomain"] = .string(nsError.domain)
        payload["errorCode"] = .integer(Int64(nsError.code))
        payload["errorType"] = .string(String(describing: type(of: error)))
        payload["errorKind"] = .string(classifyError(safeMessage))
        payload["timeout"] = .bool(isTimeout(nsError: nsError, message: safeMessage))
        payload["retriable"] = .bool(isRetriable(nsError: nsError, message: safeMessage))
        payload["error"] = .string(safeMessage)
        if let statusCode = statusCode(in: safeMessage) {
            payload["statusCode"] = .integer(Int64(statusCode))
        }
        if let offerings = capturedValues(named: "offering", in: safeMessage), !offerings.isEmpty {
            payload["offerings"] = .string(offerings.joined(separator: ","))
        }
        if let forceRegionApplied = booleanValue(named: "forceRegionApplied", in: safeMessage) {
            payload["forceRegionApplied"] = .bool(forceRegionApplied)
        }
        return payload
    }

    static func safeError(_ error: any Error) -> String {
        let nsError = error as NSError
        var value = "\(nsError.domain)#\(nsError.code): \(nsError.localizedDescription)"
        let patterns = [
            #"https?://[^\s,;]+"#,
            #"(?i)(bearer|gstoken|refresh[_ -]?token)[=: ]+[^\s,;}]+"#,
            #"cloud-[0-9a-fA-F]{16}"#,
        ]
        for pattern in patterns {
            value = value.replacingOccurrences(
                of: pattern,
                with: "<redacted>",
                options: .regularExpression
            )
        }
        return value
    }

    private static func classifyError(_ message: String) -> String {
        let normalized = message.lowercased()
        if normalized.contains("xgpuwebf2p") {
            return "xgpuwebf2p"
        }
        if normalized.contains("xgpuweb") {
            return "xgpuweb"
        }
        if normalized.contains("xcloud token") {
            return "xcloudTokenUnavailable"
        }
        if normalized.contains("cancel") || normalized.contains("取消") {
            return "cancelled"
        }
        if normalized.contains("timed out") || normalized.contains("timeout") {
            return "timeout"
        }
        return "requestFailed"
    }

    private static func isTimeout(nsError: NSError, message: String) -> Bool {
        let projected = capturedValues(named: "timeout", in: message)
        if let projected, !projected.isEmpty {
            return projected.contains { $0.caseInsensitiveCompare("true") == .orderedSame }
        }
        return nsError.domain == NSURLErrorDomain && nsError.code == NSURLErrorTimedOut
            || message.localizedCaseInsensitiveContains("timeout")
            || message.localizedCaseInsensitiveContains("timed out")
    }

    private static func isRetriable(nsError: NSError, message: String) -> Bool {
        let projected = capturedValues(named: "retriable", in: message)
        if let projected, !projected.isEmpty {
            return projected.contains { $0.caseInsensitiveCompare("true") == .orderedSame }
        }
        if nsError.domain == NSURLErrorDomain {
            return true
        }
        guard let statusCode = statusCode(in: message) else {
            return isTimeout(nsError: nsError, message: message)
        }
        return statusCode == 408 || statusCode == 429 || statusCode >= 500
    }

    private static func statusCode(in message: String) -> Int? {
        let pattern = #"(?i)(?:status(?:Code)?|http)[^0-9]{0,12}([1-5][0-9]{2})"#
        guard let expression = try? NSRegularExpression(pattern: pattern),
              let match = expression.firstMatch(
                  in: message,
                  range: NSRange(message.startIndex..., in: message)
              ),
              let range = Range(match.range(at: 1), in: message)
        else {
            return nil
        }
        return Int(message[range])
    }

    private static func capturedValues(named name: String, in message: String) -> [String]? {
        let pattern = "(?i)\\b\(name)=([a-z0-9_-]+)"
        guard let expression = try? NSRegularExpression(pattern: pattern) else {
            return nil
        }
        return expression.matches(
            in: message,
            range: NSRange(message.startIndex..., in: message)
        ).compactMap { match in
            guard let range = Range(match.range(at: 1), in: message) else { return nil }
            return String(message[range])
        }
    }

    private static func booleanValue(named name: String, in message: String) -> Bool? {
        guard let value = capturedValues(named: name, in: message)?.first else { return nil }
        switch value.lowercased() {
        case "true": return true
        case "false": return false
        default: return nil
        }
    }
}

enum CloudCatalogRefreshReason: String, Equatable, Sendable {
    case initialActivation
    case cacheMiss
    case expiredCache
    case pageEnter
    case pullToRefresh
    case manualRetry
}

@MainActor
final class CloudLibraryStore: ObservableObject {
    @Published private(set) var games: [CloudLibraryGame] = []
    @Published private(set) var phase: DataLoadPhase = .idle
    @Published private(set) var cacheState: CloudCatalogCacheState = .miss
    @Published private(set) var isRefreshing = false
    @Published private(set) var refreshReason: CloudCatalogRefreshReason?
    @Published private(set) var errorMessage: String?
    @Published private(set) var failedHydrationChunks = 0

    private let client: any XboxCloudDataClient
    private let repository: any CloudCatalogSnapshotStoring
    private var activeScope: CloudCatalogScope?
    private var activeAccess: PreparedCloudAccess?
    private var cachedSnapshot: CloudCatalogSnapshot?
    private var activitiesByTitleID: [String: GameSummary] = [:]
    private var inFlight: Task<RemoteLoadResult, Error>?
    private var inFlightOperationID: String?
    private var refreshTask: Task<Void, Never>?
    private var refreshTaskID: UUID?
    private var generation = 0
    private var didRunInitialRefresh = false

    init(
        client: any XboxCloudDataClient = RustXboxCloudDataClient(),
        repository: any CloudCatalogSnapshotStoring = CloudCatalogSnapshotRepository()
    ) {
        self.client = client
        self.repository = repository
    }

    func restoreCached(
        session: StoredAuthSession?,
        source: String = "unspecified"
    ) async {
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "cloud-library",
            event: "cacheRestoreStarted",
            payload: [
                "source": .string(source),
                "generation": .integer(Int64(generation)),
                "hasSession": .bool(session != nil),
                "hasCloudScope": .bool(
                    session?.cloudAccountID != nil && session?.cloudRegionHost != nil
                ),
            ],
            dimension: .core,
            importance: .key,
            operationID: operationID
        )
        guard let session,
              let accountID = session.cloudAccountID,
              let regionHost = session.cloudRegionHost
        else {
            if session == nil {
                IOSRuntimeTrace.decision(
                    domain: "cloud-library",
                    event: "cacheRestoreSkipped",
                    payload: ["reason": "signedOut", "action": "clear"],
                    dimension: .lifecycle,
                    importance: .key,
                    operationID: operationID
                )
                await clear()
            } else {
                IOSRuntimeTrace.decision(
                    domain: "cloud-library",
                    event: "cacheRestoreRejected",
                    payload: [
                        "reason": "scopeMissing",
                        "generation": .integer(Int64(generation)),
                    ],
                    dimension: .core,
                    importance: .key,
                    operationID: operationID
                )
            }
            return
        }
        let scope = Self.scope(accountID: accountID, regionHost: regionHost)
        guard scope != activeScope else {
            IOSRuntimeTrace.decision(
                domain: "cloud-library",
                event: "cacheRestoreSkipped",
                payload: [
                    "reason": "scopeUnchanged",
                    "generation": .integer(Int64(generation)),
                    "market": .string(scope.market),
                    "language": .string(scope.language),
                ],
                dimension: .core,
                importance: .debug,
                operationID: operationID
            )
            return
        }

        let cancelledRefresh = inFlight != nil
        let previousAccess = activeAccess
        generation += 1
        didRunInitialRefresh = false
        refreshTask?.cancel()
        refreshTask = nil
        refreshTaskID = nil
        inFlight?.cancel()
        inFlight = nil
        inFlightOperationID = nil
        isRefreshing = false
        refreshReason = nil
        IOSRuntimeTrace.decision(
            domain: "cloud-library",
            event: "cacheRestoreScopeChanged",
            payload: [
                "generation": .integer(Int64(generation)),
                "cancelledRefresh": .bool(cancelledRefresh),
                "market": .string(scope.market),
                "language": .string(scope.language),
            ],
            dimension: .lifecycle,
            importance: .key,
            operationID: operationID
        )
        activeScope = scope
        activeAccess = nil
        errorMessage = nil
        if let previousAccess {
            await client.releaseAccess(handle: previousAccess.handle)
        }

        do {
            guard let snapshot = try await repository.load(scope: scope) else {
                cachedSnapshot = nil
                games = []
                cacheState = .miss
                phase = .idle
                IOSRuntimeTrace.snapshot(
                    domain: "cloud-library",
                    event: "cacheRestoreMiss",
                    payload: [
                        "generation": .integer(Int64(generation)),
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ],
                    dimension: .core,
                    importance: .key,
                    operationID: operationID
                )
                return
            }
            cachedSnapshot = snapshot
            cacheState = snapshot.cacheState()
            applyCatalogGames(snapshot.games)
            phase = .loaded
            IOSRuntimeTrace.snapshot(
                domain: "cloud-library",
                event: "cacheRestoreHit",
                payload: [
                    "generation": .integer(Int64(generation)),
                    "cacheState": .string(snapshot.cacheState().rawValue),
                    "games": .integer(Int64(snapshot.games.count)),
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .core,
                importance: .key,
                operationID: operationID
            )
        } catch {
            cachedSnapshot = nil
            games = []
            cacheState = .miss
            phase = .failed
            errorMessage = "本地游戏库缓存读取失败"
            IOSRuntimeTrace.event(
                domain: "cloud-library",
                event: "cacheRestoreFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "generation": .integer(Int64(generation)),
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ]
                ),
                dimension: .core,
                importance: .essential,
                operationID: operationID
            )
        }
    }

    func activateOnce(
        session: StoredAuthSession?,
        prepareAccess: @escaping @MainActor () async throws -> PreparedCloudAccess
    ) async {
        IOSRuntimeTrace.event(
            domain: "cloud-library",
            event: "catalogActivationStarted",
            payload: [
                "generation": .integer(Int64(generation)),
                "phase": .string(String(describing: phase)),
                "cacheState": .string(cacheState.rawValue),
                "games": .integer(Int64(games.count)),
            ],
            dimension: .lifecycle,
            importance: .key
        )
        await restoreCached(session: session, source: "libraryActivate")
        guard session != nil else {
            IOSRuntimeTrace.decision(
                domain: "cloud-library",
                event: "catalogActivationSkipped",
                payload: ["reason": "signedOut"],
                dimension: .lifecycle,
                importance: .key
            )
            return
        }
        guard !didRunInitialRefresh else {
            IOSRuntimeTrace.decision(
                domain: "cloud-library",
                event: "catalogActivationSkipped",
                payload: [
                    "reason": "alreadyActivated",
                    "cacheState": .string(cacheState.rawValue),
                    "games": .integer(Int64(games.count)),
                ],
                dimension: .core,
                importance: .key
            )
            return
        }
        didRunInitialRefresh = true
        IOSRuntimeTrace.decision(
            domain: "cloud-library",
            event: "catalogActivationRefreshRequired",
            payload: [
                "reason": "initialActivation",
                "cacheState": .string(cacheState.rawValue),
            ],
            dimension: .core,
            importance: .key
        )
        await refresh(reason: .initialActivation, prepareAccess: prepareAccess)
    }

    func activate(
        session: StoredAuthSession?,
        prepareAccess: @escaping @MainActor () async throws -> PreparedCloudAccess
    ) async {
        await activateOnce(session: session, prepareAccess: prepareAccess)
    }

    func refresh(
        reason: CloudCatalogRefreshReason,
        prepareAccess: @escaping @MainActor () async throws -> PreparedCloudAccess
    ) async {
        if let refreshTask {
            IOSRuntimeTrace.decision(
                domain: "cloud-library",
                event: "catalogRefreshCoalesced",
                payload: [
                    "generation": .integer(Int64(generation)),
                    "reason": .string(reason.rawValue),
                ],
                dimension: .lifecycle,
                importance: .key,
                operationID: inFlightOperationID
            )
            await refreshTask.value
            return
        }

        let taskID = UUID()
        let task = Task { @MainActor [weak self] in
            guard let self else { return }
            await self.performRefresh(reason: reason, prepareAccess: prepareAccess)
        }
        refreshTask = task
        refreshTaskID = taskID
        await task.value
        if refreshTaskID == taskID {
            refreshTask = nil
            refreshTaskID = nil
        }
    }

    private func performRefresh(
        reason: CloudCatalogRefreshReason,
        prepareAccess: @escaping @MainActor () async throws -> PreparedCloudAccess
    ) async {

        let requestGeneration = generation
        let market = Self.market
        let language = Self.language
        let existingAccess = activeAccess
        let startedAt = Date()
        let operationID = UUID().uuidString
        IOSRuntimeTrace.event(
            domain: "cloud-library",
            event: "catalogRefreshStarted",
            payload: [
                "generation": .integer(Int64(requestGeneration)),
                "reason": .string(reason.rawValue),
                "cachedGames": .integer(Int64(games.count)),
                "reuseAccess": .bool(existingAccess != nil),
                "market": .string(market),
                "language": .string(language),
            ],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
        let task = Task { @MainActor [client] in
            let access: PreparedCloudAccess
            if let existingAccess {
                access = existingAccess
            } else {
                access = try await prepareAccess()
            }
            do {
                let snapshot = try await client.loadCatalog(
                    accessHandle: access.handle,
                    market: market,
                    language: language
                )
                return RemoteLoadResult(access: access, snapshot: snapshot)
            } catch {
                IOSRuntimeTrace.decision(
                    domain: "cloud-library",
                    event: "catalogAccessRenewRequired",
                    payload: ["reason": "catalogRequestFailed"],
                    dimension: .recovery,
                    importance: .key,
                    operationID: operationID
                )
                await client.releaseAccess(handle: access.handle)
                let renewedAccess = try await prepareAccess()
                let snapshot = try await client.loadCatalog(
                    accessHandle: renewedAccess.handle,
                    market: market,
                    language: language
                )
                return RemoteLoadResult(access: renewedAccess, snapshot: snapshot)
            }
        }
        inFlight = task
        inFlightOperationID = operationID
        refreshReason = reason
        isRefreshing = !games.isEmpty
        errorMessage = nil
        if games.isEmpty {
            phase = .loading
        }

        do {
            let result = try await task.value
            guard requestGeneration == generation, !Task.isCancelled else {
                IOSRuntimeTrace.decision(
                    domain: "cloud-library",
                    event: "catalogRefreshDiscarded",
                    payload: [
                        "requestGeneration": .integer(Int64(requestGeneration)),
                        "currentGeneration": .integer(Int64(generation)),
                        "taskCancelled": .bool(Task.isCancelled),
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ],
                    dimension: .lifecycle,
                    importance: .key,
                    operationID: operationID
                )
                inFlight = nil
                inFlightOperationID = nil
                return
            }
            IOSRuntimeTrace.snapshot(
                domain: "cloud-library",
                event: "catalogRemoteSnapshotReceived",
                payload: [
                    "generation": .integer(Int64(requestGeneration)),
                    "games": .integer(Int64(result.snapshot.games.count)),
                    "pendingHydration": .integer(Int64(result.snapshot.pendingHydrationProductIDs.count)),
                    "failedHydrationChunks": .integer(Int64(result.snapshot.failedHydrationChunks)),
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            activeAccess = result.access
            activeScope = result.snapshot.scope
            failedHydrationChunks = result.snapshot.failedHydrationChunks
            let mergedGames = mergeActivities(into: result.snapshot.games)
            let previousImageURLs = cachedSnapshot?.successfulImageURLs ?? [:]
            let snapshot = CloudCatalogSnapshot(
                scope: result.snapshot.scope,
                games: mergedGames,
                baseUpdatedAt: result.snapshot.fetchedAt,
                overlayUpdatedAt: result.snapshot.fetchedAt,
                successfulImageURLs: previousImageURLs
            )
            try await repository.save(snapshot)
            cachedSnapshot = snapshot
            cacheState = .fresh
            let didPublishSnapshot = games != mergedGames
            if didPublishSnapshot {
                games = mergedGames
            }
            phase = .loaded
            IOSRuntimeTrace.state(
                domain: "cloud-library",
                event: "catalogRefreshCommitted",
                payload: [
                    "generation": .integer(Int64(requestGeneration)),
                    "games": .integer(Int64(games.count)),
                    "published": .bool(didPublishSnapshot),
                    "cacheState": .string(cacheState.rawValue),
                ],
                dimension: .core,
                importance: .key,
                operationID: operationID
            )
            failedHydrationChunks += await hydrateMetadataPages(
                productIDs: result.snapshot.pendingHydrationProductIDs,
                access: result.access,
                scope: result.snapshot.scope,
                fetchedAt: result.snapshot.fetchedAt,
                requestGeneration: requestGeneration,
                operationID: operationID
            )
            if failedHydrationChunks > 0 {
                errorMessage = "部分游戏图片和详情将在稍后补齐"
            }
        } catch is CancellationError {
            IOSRuntimeTrace.decision(
                domain: "cloud-library",
                event: "catalogRefreshCancelled",
                payload: [
                    "generation": .integer(Int64(requestGeneration)),
                    "currentGeneration": .integer(Int64(generation)),
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .lifecycle,
                importance: .key,
                operationID: operationID
            )
        } catch {
            guard requestGeneration == generation else {
                IOSRuntimeTrace.decision(
                    domain: "cloud-library",
                    event: "catalogRefreshDiscarded",
                    payload: CloudLibraryDiagnostics.errorPayload(
                        error,
                        extra: [
                            "requestGeneration": .integer(Int64(requestGeneration)),
                            "currentGeneration": .integer(Int64(generation)),
                            "stage": "error",
                            "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                        ]
                    ),
                    dimension: .lifecycle,
                    importance: .key,
                    operationID: operationID
                )
                inFlight = nil
                inFlightOperationID = nil
                return
            }
            errorMessage = games.isEmpty
                ? "无法载入云游戏目录，请稍后重试"
                : "刷新失败，当前继续显示缓存内容"
            phase = games.isEmpty ? .failed : .loaded
            IOSRuntimeTrace.event(
                domain: "cloud-library",
                event: "catalogRefreshFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "generation": .integer(Int64(requestGeneration)),
                        "cachedGames": .integer(Int64(games.count)),
                        "phase": .string(String(describing: phase)),
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ]
                ),
                dimension: .network,
                importance: .essential,
                operationID: operationID
            )
        }

        inFlight = nil
        inFlightOperationID = nil
        isRefreshing = false
        refreshReason = nil
        IOSRuntimeTrace.state(
            domain: "cloud-library",
            event: "libraryStateChanged",
            payload: [
                "generation": .integer(Int64(requestGeneration)),
                "currentGeneration": .integer(Int64(generation)),
                "phase": .string(String(describing: phase)),
                "games": .integer(Int64(games.count)),
                "failedHydrationChunks": .integer(Int64(failedHydrationChunks)),
                "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
            ],
            dimension: .core,
            importance: .key,
            operationID: operationID
        )
    }

    func updateActivities(_ activities: [GameSummary]) {
        IOSRuntimeTrace.snapshot(
            domain: "cloud-library",
            event: "activityOverlayStarted",
            payload: [
                "activities": .integer(Int64(activities.count)),
                "catalogGames": .integer(Int64(games.count)),
            ],
            dimension: .core,
            importance: .debug
        )
        activitiesByTitleID = Dictionary(
            uniqueKeysWithValues: activities.map { ($0.titleID, $0) }
        )
        let merged = mergeActivities(into: cachedSnapshot?.games ?? games)
        if merged != games {
            games = merged
            IOSRuntimeTrace.state(
                domain: "cloud-library",
                event: "activityOverlayCommitted",
                payload: [
                    "activities": .integer(Int64(activities.count)),
                    "games": .integer(Int64(games.count)),
                ],
                dimension: .core,
                importance: .key
            )
        }
    }

    func recordSuccessfulImage(productID: String, url: URL) {
        guard let snapshot = cachedSnapshot,
              snapshot.successfulImageURLs[productID] != url
        else {
            return
        }
        let updated = snapshot.withSuccessfulImage(productID: productID, url: url)
        cachedSnapshot = updated
        IOSRuntimeTrace.state(
            domain: "image",
            event: "preferredImageUpdated",
            payload: [
                "successfulImages": .integer(Int64(updated.successfulImageURLs.count)),
                "scheme": .string(url.scheme ?? "unknown"),
            ],
            dimension: .presentation,
            importance: .debug
        )
        Task {
            try? await repository.save(updated)
        }
    }

    func preferredImageURL(for game: CloudLibraryGame) -> URL? {
        cachedSnapshot?.successfulImageURLs[game.productID]
    }

    func clear() async {
        let operationID = UUID().uuidString
        IOSRuntimeTrace.event(
            domain: "cloud-library",
            event: "libraryClearStarted",
            payload: [
                "generation": .integer(Int64(generation)),
                "games": .integer(Int64(games.count)),
                "hasRefresh": .bool(inFlight != nil),
                "hasAccess": .bool(activeAccess != nil),
            ],
            dimension: .lifecycle,
            importance: .key,
            operationID: operationID
        )
        generation += 1
        didRunInitialRefresh = false
        refreshTask?.cancel()
        refreshTask = nil
        refreshTaskID = nil
        inFlight?.cancel()
        inFlight = nil
        inFlightOperationID = nil
        let accountID = activeScope?.accountID
        if let activeAccess {
            await client.releaseAccess(handle: activeAccess.handle)
        }
        if let accountID {
            try? await repository.clearOverlay(accountID: accountID)
        }
        activeAccess = nil
        activeScope = nil
        cachedSnapshot = nil
        activitiesByTitleID = [:]
        games = []
        phase = .idle
        cacheState = .miss
        isRefreshing = false
        refreshReason = nil
        errorMessage = nil
        failedHydrationChunks = 0
        IOSRuntimeTrace.state(
            domain: "cloud-library",
            event: "libraryStateChanged",
            payload: [
                "generation": .integer(Int64(generation)),
                "phase": "idle",
                "cacheState": "miss",
                "games": 0,
            ],
            dimension: .core,
            importance: .key,
            operationID: operationID
        )
    }

    private func applyCatalogGames(_ catalogGames: [CloudLibraryGame]) {
        let merged = mergeActivities(into: catalogGames)
        if games != merged {
            games = merged
        }
    }

    private func hydrateMetadataPages(
        productIDs: [String],
        access: PreparedCloudAccess,
        scope: CloudCatalogScope,
        fetchedAt: Date,
        requestGeneration: Int,
        operationID: String
    ) async -> Int {
        var failedPages = 0
        let pages = productIDs.chunked(maxCount: 75)
        guard !pages.isEmpty else {
            return 0
        }
        IOSRuntimeTrace.event(
            domain: "cloud-library",
            event: "metadataHydrationStarted",
            payload: [
                "generation": .integer(Int64(requestGeneration)),
                "products": .integer(Int64(productIDs.count)),
                "pages": .integer(Int64(pages.count)),
            ],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
        for (pageIndex, page) in pages.enumerated() {
            guard requestGeneration == generation, !Task.isCancelled else {
                IOSRuntimeTrace.decision(
                    domain: "cloud-library",
                    event: "metadataHydrationStopped",
                    payload: [
                        "requestGeneration": .integer(Int64(requestGeneration)),
                        "currentGeneration": .integer(Int64(generation)),
                        "pageIndex": .integer(Int64(pageIndex)),
                        "pageCount": .integer(Int64(pages.count)),
                        "taskCancelled": .bool(Task.isCancelled),
                    ],
                    dimension: .lifecycle,
                    importance: .key,
                    operationID: operationID
                )
                break
            }
            let pageStartedAt = Date()
            IOSRuntimeTrace.event(
                domain: "cloud-library",
                event: "metadataPageStarted",
                payload: [
                    "pageIndex": .integer(Int64(pageIndex)),
                    "pageCount": .integer(Int64(pages.count)),
                    "requested": .integer(Int64(page.count)),
                ],
                dimension: .network,
                importance: .debug,
                operationID: operationID
            )
            do {
                let metadata = try await client.loadMetadataPage(
                    accessHandle: access.handle,
                    market: scope.market,
                    language: scope.language,
                    productIDs: page
                )
                guard requestGeneration == generation else {
                    IOSRuntimeTrace.decision(
                        domain: "cloud-library",
                        event: "metadataPageDiscarded",
                        payload: [
                            "requestGeneration": .integer(Int64(requestGeneration)),
                            "currentGeneration": .integer(Int64(generation)),
                            "pageIndex": .integer(Int64(pageIndex)),
                        ],
                        dimension: .lifecycle,
                        importance: .key,
                        operationID: operationID
                    )
                    break
                }
                let currentGames = cachedSnapshot?.games ?? games
                let updatedGames = merge(metadata: metadata, into: currentGames)
                guard updatedGames != currentGames else {
                    IOSRuntimeTrace.snapshot(
                        domain: "cloud-library",
                        event: "metadataPageUnchanged",
                        payload: [
                            "pageIndex": .integer(Int64(pageIndex)),
                            "pageCount": .integer(Int64(pages.count)),
                            "requested": .integer(Int64(page.count)),
                            "received": .integer(Int64(metadata.count)),
                            "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: pageStartedAt))),
                        ],
                        dimension: .network,
                        importance: .debug,
                        operationID: operationID
                    )
                    continue
                }
                let snapshot = CloudCatalogSnapshot(
                    scope: scope,
                    games: updatedGames,
                    baseUpdatedAt: fetchedAt,
                    overlayUpdatedAt: fetchedAt,
                    successfulImageURLs: cachedSnapshot?.successfulImageURLs ?? [:]
                )
                cachedSnapshot = snapshot
                try await repository.save(snapshot)
                applyCatalogGames(updatedGames)
                IOSRuntimeTrace.state(
                    domain: "cloud-library",
                    event: "metadataPageCommitted",
                    payload: [
                        "pageIndex": .integer(Int64(pageIndex)),
                        "pageCount": .integer(Int64(pages.count)),
                        "requested": .integer(Int64(page.count)),
                        "received": .integer(Int64(metadata.count)),
                        "games": .integer(Int64(updatedGames.count)),
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: pageStartedAt))),
                    ],
                    dimension: .core,
                    importance: .key,
                    operationID: operationID
                )
                await Task.yield()
            } catch is CancellationError {
                IOSRuntimeTrace.decision(
                    domain: "cloud-library",
                    event: "metadataPageCancelled",
                    payload: [
                        "pageIndex": .integer(Int64(pageIndex)),
                        "pageCount": .integer(Int64(pages.count)),
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: pageStartedAt))),
                    ],
                    dimension: .lifecycle,
                    importance: .key,
                    operationID: operationID
                )
                break
            } catch {
                failedPages += 1
                IOSRuntimeTrace.event(
                    domain: "cloud-library",
                    event: "metadataPageFailed",
                    payload: CloudLibraryDiagnostics.errorPayload(
                        error,
                        extra: [
                            "pageIndex": .integer(Int64(pageIndex)),
                            "pageCount": .integer(Int64(pages.count)),
                            "requested": .integer(Int64(page.count)),
                            "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: pageStartedAt))),
                        ]
                    ),
                    dimension: .network,
                    importance: .key,
                    operationID: operationID
                )
            }
        }
        IOSRuntimeTrace.snapshot(
            domain: "cloud-library",
            event: "metadataHydrationCompleted",
            payload: [
                "generation": .integer(Int64(requestGeneration)),
                "pages": .integer(Int64(pages.count)),
                "failedPages": .integer(Int64(failedPages)),
            ],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
        return failedPages
    }

    private func merge(
        metadata: [CloudCatalogMetadata],
        into catalogGames: [CloudLibraryGame]
    ) -> [CloudLibraryGame] {
        let metadataByProductID = Dictionary(
            uniqueKeysWithValues: metadata.map { ($0.productID, $0) }
        )
        return catalogGames.map { game in
            guard let value = metadataByProductID[game.productID] else {
                return game
            }
            return CloudLibraryGame(
                productID: game.productID,
                streamTitleID: game.streamTitleID,
                xboxTitleID: game.xboxTitleID,
                name: value.name.isEmpty ? game.name : value.name,
                publisherName: value.publisherName.isEmpty
                    ? game.publisherName
                    : value.publisherName,
                description: value.description.isEmpty ? game.description : value.description,
                tileURL: value.tileURL ?? game.tileURL,
                posterURL: value.posterURL ?? game.posterURL,
                heroURL: value.heroURL ?? game.heroURL,
                artworkURL: game.artworkURL,
                categories: value.categories.isEmpty ? game.categories : value.categories,
                supportedInputTypes: game.supportedInputTypes,
                hasEntitlement: game.hasEntitlement,
                isRecentlyPlayed: game.isRecentlyPlayed,
                isNew: game.isNew,
                lastPlayedAt: game.lastPlayedAt,
                playtimeMinutes: game.playtimeMinutes,
                achievementProgress: game.achievementProgress
            )
        }
    }

    private func mergeActivities(into catalogGames: [CloudLibraryGame]) -> [CloudLibraryGame] {
        catalogGames.map { game in
            guard let xboxTitleID = game.xboxTitleID,
                  let activity = activitiesByTitleID[xboxTitleID]
            else {
                return game
            }
            return CloudLibraryGame(
                productID: game.productID,
                streamTitleID: game.streamTitleID,
                xboxTitleID: game.xboxTitleID,
                name: game.name,
                publisherName: game.publisherName,
                description: game.description,
                tileURL: game.tileURL,
                posterURL: game.posterURL,
                heroURL: game.heroURL,
                artworkURL: activity.artworkURL ?? game.artworkURL,
                categories: game.categories,
                supportedInputTypes: game.supportedInputTypes,
                hasEntitlement: game.hasEntitlement,
                isRecentlyPlayed: game.isRecentlyPlayed,
                isNew: game.isNew,
                lastPlayedAt: activity.lastPlayedAt,
                playtimeMinutes: activity.playtimeMinutes,
                achievementProgress: activity.achievementProgress
            )
        }
    }

    private static func scope(accountID: String, regionHost: String) -> CloudCatalogScope {
        CloudCatalogScope(
            accountID: accountID,
            regionHost: regionHost,
            language: language,
            market: market
        )
    }

    static func catalogLanguage(preferredLanguage: String?) -> String {
        let normalized = preferredLanguage?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased() ?? ""
        return normalized.hasPrefix("zh") ? "zh-TW" : "en-US"
    }

    static func catalogMarket() -> String {
        "US"
    }

    private static var language: String {
        // 与桌面 XCloud 页保持同一目录查询合同，避免设备 Locale 直接改变目录集合。
        catalogLanguage(preferredLanguage: Locale.preferredLanguages.first)
    }

    private static var market: String {
        catalogMarket()
    }
}

private struct RemoteLoadResult: Sendable {
    let access: PreparedCloudAccess
    let snapshot: RemoteCloudCatalogSnapshot
}

private extension Array {
    func chunked(maxCount: Int) -> [[Element]] {
        guard maxCount > 0 else {
            return []
        }
        return stride(from: 0, to: count, by: maxCount).map { start in
            Array(self[start..<Swift.min(start + maxCount, count)])
        }
    }
}
