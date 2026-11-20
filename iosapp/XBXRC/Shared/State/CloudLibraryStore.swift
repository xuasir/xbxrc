import Combine
import Foundation

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
    @Published private(set) var unavailableSources: Set<String> = []
    @Published var sourceStates: [String: XboxSingleDataState<Bool>] = [:]
    @Published var catalogFailure: SectionRequestFailure?
    @Published var catalogUpdatedAt: Date?
    @Published var metadataState = XboxSingleDataState<Bool>()
    private var pendingMetadataIDs: Set<String> = []
    @Published var productDetails: [String: XboxSingleDataState<CloudProductDetails>] = [:]
    @Published var waitTimes: [String: XboxSingleDataState<UInt64>] = [:]
    @Published private(set) var productQueryGeneration = 0
    var productRequestIDs: [String: UUID] = [:]

    weak var authentication: AuthStore?
    private let client: any XboxCloudDataClient
    private let settingsProvider: AppSettingsStore?
    private let repository: any CloudCatalogSnapshotStoring
    private let persistenceCoordinator: CloudCatalogPersistenceCoordinator
    private var activeScope: CloudCatalogScope?
    private var activeAccess: PreparedCloudAccess?
    private var cachedSnapshot: CloudCatalogSnapshot?
    private var activitiesByTitleID: [String: GameSummary] = [:]
    private var inFlight: Task<RemoteLoadResult, Error>?
    private var inFlightOperationID: String?
    private var refreshTask: Task<Void, Never>?
    private var refreshTaskID: UUID?
    private var sourceCommitTask: Task<Void, Error>?
    private var sourceCommitTaskID: UUID?
    private var generation = 0
    private var didRunInitialRefresh = false

    init(
        client: any XboxCloudDataClient = RustXboxCloudDataClient(),
        repository: any CloudCatalogSnapshotStoring = CloudCatalogSnapshotRepository(),
        settingsProvider: AppSettingsStore? = nil
    ) {
        self.client = client
        self.settingsProvider = settingsProvider
        self.repository = repository
        persistenceCoordinator = CloudCatalogPersistenceCoordinator(repository: repository)
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
                let transition = beginScopeTransition(to: nil)
                await settleScopeTransition(transition, clearOverlay: false)
            }
            return
        }
        let scope = scope(accountID: accountID, regionHost: regionHost)
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

        let transition = beginScopeTransition(to: scope)
        let revision = CloudLibraryScopeRevision(generation: generation, scope: scope)
        IOSRuntimeTrace.decision(
            domain: "cloud-library",
            event: "cacheRestoreScopeChanged",
            payload: [
                "generation": .integer(Int64(generation)),
                "cancelledRefresh": .bool(transition.cancelledRefresh),
                "market": .string(scope.market),
                "language": .string(scope.language),
            ],
            dimension: .lifecycle,
            importance: .key,
            operationID: operationID
        )
        await settleScopeTransition(transition, clearOverlay: false)
        guard isCurrent(revision) else { return }

        do {
            guard let snapshot = try await repository.load(scope: scope) else {
                guard isCurrent(revision) else { return }
                cachedSnapshot = nil
                games = []
                cacheState = .miss
                phase = .idle
                IOSRuntimeTrace.snapshot(
                    domain: "cloud-library",
                    event: "cacheRestoreMiss",
                    payload: [
                        "generation": .integer(Int64(generation)),
                        "elapsedMs": .integer(
                            Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ],
                    dimension: .core,
                    importance: .key,
                    operationID: operationID
                )
                return
            }
            guard isCurrent(revision), snapshot.scope == scope else { return }
            cachedSnapshot = snapshot
            cacheState = snapshot.cacheState()
            unavailableSources = snapshot.unavailableSources ?? []
            catalogUpdatedAt = snapshot.baseUpdatedAt
            for source in ["recent", "newest", "popular", "leaving"] {
                let failed = unavailableSources.contains(source)
                let date =
                    snapshot.sourceUpdatedAt?[source] ?? (failed ? nil : snapshot.overlayUpdatedAt)
                sourceStates[source] = XboxSingleDataState(
                    value: date == nil ? nil : true, phase: date == nil ? .failed : .loaded,
                    updatedAt: date,
                    failure: failed ? SectionRequestFailure(kind: .unknown) : nil,
                    requestAttempt: .refresh)
            }
            applyCatalogGames(snapshot.games)
            phase = .loaded
            IOSRuntimeTrace.snapshot(
                domain: "cloud-library",
                event: "cacheRestoreHit",
                payload: [
                    "generation": .integer(Int64(generation)),
                    "cacheState": .string(snapshot.cacheState().rawValue),
                    "games": .integer(Int64(snapshot.games.count)),
                    "elapsedMs": .integer(
                        Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .core,
                importance: .key,
                operationID: operationID
            )
        } catch {
            guard isCurrent(revision) else { return }
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
                        "elapsedMs": .integer(
                            Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
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
        let requestRevision = CloudLibraryScopeRevision(
            generation: generation,
            scope: activeScope
        )
        // 全页刷新接管栏目状态，等待已有缓存提交收尾，再提交新快照。
        let previousCommit = sourceCommitTask
        productRequestIDs = productRequestIDs.filter { !$0.key.hasPrefix("catalog-source-") }
        previousCommit?.cancel()
        _ = try? await previousCommit?.value
        guard isCurrent(requestRevision), !Task.isCancelled else { return }
        let market = self.market
        let language = self.language
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
                guard SectionRequestFailure(error: error).kind == .authentication else {
                    throw error
                }
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
        isRefreshing = true
        catalogFailure = nil
        for source in ["recent", "newest", "popular", "leaving"] {
            sourceStates[source, default: XboxSingleDataState()].beginRequest()
        }
        errorMessage = nil
        if games.isEmpty {
            phase = .loading
        }

        do {
            let result = try await task.value
            guard isCurrent(requestRevision),
                !Task.isCancelled,
                requestRevision.scope == nil || requestRevision.scope == result.snapshot.scope
            else {
                IOSRuntimeTrace.decision(
                    domain: "cloud-library",
                    event: "catalogRefreshDiscarded",
                    payload: [
                        "requestGeneration": .integer(Int64(requestGeneration)),
                        "currentGeneration": .integer(Int64(generation)),
                        "taskCancelled": .bool(Task.isCancelled),
                        "elapsedMs": .integer(
                            Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ],
                    dimension: .lifecycle,
                    importance: .key,
                    operationID: operationID
                )
                if existingAccess?.handle != result.access.handle {
                    await client.releaseAccess(handle: result.access.handle)
                }
                clearRefreshOwnership(operationID: operationID)
                return
            }
            IOSRuntimeTrace.snapshot(
                domain: "cloud-library",
                event: "catalogRemoteSnapshotReceived",
                payload: [
                    "generation": .integer(Int64(requestGeneration)),
                    "games": .integer(Int64(result.snapshot.games.count)),
                    "pendingHydration": .integer(
                        Int64(result.snapshot.pendingHydrationProductIDs.count)),
                    "failedHydrationChunks": .integer(Int64(result.snapshot.failedHydrationChunks)),
                    "elapsedMs": .integer(
                        Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            let sourceGames = preserveDiscoveryRanks(
                result.snapshot.games, unavailable: result.snapshot.unavailableSources)
            let mergedGames = mergeActivities(into: sourceGames)
            let previousImageURLs = cachedSnapshot?.successfulImageURLs ?? [:]
            let snapshot = CloudCatalogSnapshot(
                scope: result.snapshot.scope,
                games: mergedGames,
                baseUpdatedAt: result.snapshot.fetchedAt,
                overlayUpdatedAt: result.snapshot.fetchedAt,
                successfulImageURLs: previousImageURLs,
                unavailableSources: result.snapshot.unavailableSources,
                sourceUpdatedAt: Dictionary(
                    ["recent", "newest", "popular", "leaving"].compactMap { source in
                        let date =
                            result.snapshot.unavailableSources.contains(source)
                            ? sourceStates[source]?.updatedAt : result.snapshot.fetchedAt
                        return date.map { (source, $0) }
                    }, uniquingKeysWith: { first, _ in first })
            )
            try await repository.save(snapshot)
            guard isCurrent(requestRevision), !Task.isCancelled else {
                await discardPersistedOverlayIfOwnerChanged(scope: snapshot.scope)
                if existingAccess?.handle != result.access.handle {
                    await client.releaseAccess(handle: result.access.handle)
                }
                clearRefreshOwnership(operationID: operationID)
                return
            }
            let committedRevision = CloudLibraryScopeRevision(
                generation: requestGeneration,
                scope: result.snapshot.scope
            )
            activeAccess = result.access
            activeScope = result.snapshot.scope
            persistenceCoordinator.synchronize(to: committedRevision)
            failedHydrationChunks = result.snapshot.failedHydrationChunks
            pendingMetadataIDs = Set(result.snapshot.pendingHydrationProductIDs)
            metadataState.beginRequest()
            unavailableSources = result.snapshot.unavailableSources
            catalogUpdatedAt = result.snapshot.fetchedAt
            catalogFailure = nil
            for source in ["recent", "newest", "popular", "leaving"] {
                var state = sourceStates[source] ?? XboxSingleDataState<Bool>()
                if unavailableSources.contains(source) {
                    state.failure =
                        result.snapshot.sourceFailures[source]
                        ?? SectionRequestFailure(kind: .unknown)
                    state.errorMessage = state.failure?.message
                    state.phase = state.hasSnapshot ? .loaded : .failed
                    state.requestAttempt = state.hasSnapshot ? .refresh : .initial
                } else {
                    state.value = true
                    state.phase = .loaded
                    state.updatedAt = result.snapshot.fetchedAt
                    state.failure = nil
                    state.errorMessage = nil
                }
                state.isRefreshing = false
                sourceStates[source] = state
            }
            cachedSnapshot = CloudCatalogSnapshot(
                scope: snapshot.scope,
                games: snapshot.games,
                baseUpdatedAt: snapshot.baseUpdatedAt,
                overlayUpdatedAt: snapshot.overlayUpdatedAt,
                successfulImageURLs: cachedSnapshot?.successfulImageURLs
                    ?? snapshot.successfulImageURLs,
                unavailableSources: snapshot.unavailableSources ?? [],
                sourceUpdatedAt: snapshot.sourceUpdatedAt
            )
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
            let hydrationFailures = await hydrateMetadataPages(
                productIDs: result.snapshot.pendingHydrationProductIDs,
                access: result.access,
                scope: result.snapshot.scope,
                fetchedAt: result.snapshot.fetchedAt,
                requestGeneration: requestGeneration,
                operationID: operationID
            )
            guard isCurrent(committedRevision) else {
                clearRefreshOwnership(operationID: operationID)
                return
            }
            failedHydrationChunks += hydrationFailures
            metadataState.isRefreshing = false
            if failedHydrationChunks > 0 {
                metadataState.failure =
                    metadataState.failure ?? SectionRequestFailure(kind: .unknown)
                metadataState.phase = metadataState.hasSnapshot ? .loaded : .failed
            } else {
                metadataState.value = true
                metadataState.phase = .loaded
                metadataState.updatedAt = .now
                metadataState.failure = nil
            }
        } catch is CancellationError {
            IOSRuntimeTrace.decision(
                domain: "cloud-library",
                event: "catalogRefreshCancelled",
                payload: [
                    "generation": .integer(Int64(requestGeneration)),
                    "currentGeneration": .integer(Int64(generation)),
                    "elapsedMs": .integer(
                        Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .lifecycle,
                importance: .key,
                operationID: operationID
            )
        } catch {
            guard isCurrent(requestRevision) else {
                IOSRuntimeTrace.decision(
                    domain: "cloud-library",
                    event: "catalogRefreshDiscarded",
                    payload: CloudLibraryDiagnostics.errorPayload(
                        error,
                        extra: [
                            "requestGeneration": .integer(Int64(requestGeneration)),
                            "currentGeneration": .integer(Int64(generation)),
                            "stage": "error",
                            "elapsedMs": .integer(
                                Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))
                            ),
                        ]
                    ),
                    dimension: .lifecycle,
                    importance: .key,
                    operationID: operationID
                )
                clearRefreshOwnership(operationID: operationID)
                return
            }
            errorMessage =
                games.isEmpty
                ? "无法载入云游戏目录，请稍后重试"
                : "刷新失败，当前继续显示缓存内容"
            catalogFailure =
                SectionRequestFailure.isCancellation(error)
                ? nil : SectionRequestFailure(error: error)
            if catalogFailure?.kind == .authentication { authentication?.requireDataSignIn() }
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
                        "elapsedMs": .integer(
                            Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ]
                ),
                dimension: .network,
                importance: .essential,
                operationID: operationID
            )
        }

        guard inFlightOperationID == operationID else { return }
        clearRefreshOwnership(operationID: operationID)
        IOSRuntimeTrace.state(
            domain: "cloud-library",
            event: "libraryStateChanged",
            payload: [
                "generation": .integer(Int64(requestGeneration)),
                "currentGeneration": .integer(Int64(generation)),
                "phase": .string(String(describing: phase)),
                "games": .integer(Int64(games.count)),
                "failedHydrationChunks": .integer(Int64(failedHydrationChunks)),
                "elapsedMs": .integer(
                    Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
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
            let scope = activeScope,
            snapshot.scope == scope,
            snapshot.successfulImageURLs[productID] != url
        else {
            return
        }
        let updated = snapshot.withSuccessfulImage(productID: productID, url: url)
        cachedSnapshot = updated
        let revision = CloudLibraryScopeRevision(generation: generation, scope: scope)
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
        persistenceCoordinator.persistSuccessfulImagePreferences(
            updated,
            revision: revision
        )
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
        let transition = beginScopeTransition(to: nil)
        let revision = CloudLibraryScopeRevision(generation: generation, scope: nil)
        await settleScopeTransition(transition, clearOverlay: true)
        guard isCurrent(revision) else { return }
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

    private func beginScopeTransition(
        to scope: CloudCatalogScope?
    ) -> CloudLibraryScopeTransition {
        generation += 1
        productQueryGeneration = generation
        productRequestIDs = [:]
        productDetails = [:]
        waitTimes = [:]
        unavailableSources = []
        sourceStates = [:]
        catalogFailure = nil
        catalogUpdatedAt = nil
        metadataState = XboxSingleDataState()
        pendingMetadataIDs = []
        let revision = CloudLibraryScopeRevision(generation: generation, scope: scope)
        let previousRefreshTask = refreshTask
        let transition = CloudLibraryScopeTransition(
            previousAccess: activeAccess,
            previousAccountID: activeScope?.accountID,
            persistenceDrain: persistenceCoordinator.transition(to: revision),
            refreshTask: previousRefreshTask,
            cancelledRefresh: inFlight != nil || refreshTask != nil
        )
        didRunInitialRefresh = false
        previousRefreshTask?.cancel()
        refreshTask = nil
        refreshTaskID = nil
        inFlight?.cancel()
        inFlight = nil
        inFlightOperationID = nil
        activeAccess = nil
        activeScope = scope
        cachedSnapshot = nil
        activitiesByTitleID = [:]
        games = []
        phase = .idle
        cacheState = .miss
        isRefreshing = false
        refreshReason = nil
        errorMessage = nil
        failedHydrationChunks = 0
        return transition
    }

    private func settleScopeTransition(
        _ transition: CloudLibraryScopeTransition,
        clearOverlay: Bool
    ) async {
        await persistenceCoordinator.settle(transition.persistenceDrain)
        await transition.refreshTask?.value
        if let previousAccess = transition.previousAccess {
            await client.releaseAccess(handle: previousAccess.handle)
        }
        if clearOverlay,
            let accountID = transition.previousAccountID,
            activeScope?.accountID != accountID
        {
            try? await repository.clearOverlay(accountID: accountID)
        }
    }

    private func isCurrent(_ revision: CloudLibraryScopeRevision) -> Bool {
        revision.generation == generation && revision.scope == activeScope
    }

    private func clearRefreshOwnership(operationID: String) {
        guard inFlightOperationID == operationID else { return }
        inFlight = nil
        inFlightOperationID = nil
        isRefreshing = false
        for source in sourceStates.keys {
            if sourceStates[source]?.isRefreshing == true {
                sourceStates[source]?.failRequest(CancellationError())
            }
        }
        if metadataState.isRefreshing { metadataState.failRequest(CancellationError()) }
        refreshReason = nil
    }

    private func discardPersistedOverlayIfOwnerChanged(
        scope: CloudCatalogScope
    ) async {
        guard activeScope?.accountID != scope.accountID else { return }
        try? await repository.clearOverlay(accountID: scope.accountID)
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
        let revision = CloudLibraryScopeRevision(
            generation: requestGeneration,
            scope: scope
        )
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
            guard isCurrent(revision), !Task.isCancelled else {
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
                guard isCurrent(revision) else {
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
                pendingMetadataIDs.subtract(page)
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
                            "elapsedMs": .integer(
                                Int64(
                                    CloudLibraryDiagnostics.elapsedMilliseconds(
                                        since: pageStartedAt))),
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
                    successfulImageURLs: cachedSnapshot?.successfulImageURLs ?? [:],
                    unavailableSources: unavailableSources,
                    sourceUpdatedAt: cachedSnapshot?.sourceUpdatedAt
                )
                try await repository.save(snapshot)
                guard isCurrent(revision), !Task.isCancelled else {
                    await discardPersistedOverlayIfOwnerChanged(scope: snapshot.scope)
                    IOSRuntimeTrace.decision(
                        domain: "cloud-library",
                        event: "metadataPageDiscarded",
                        payload: [
                            "requestGeneration": .integer(Int64(requestGeneration)),
                            "currentGeneration": .integer(Int64(generation)),
                            "pageIndex": .integer(Int64(pageIndex)),
                            "stage": "afterSave",
                        ],
                        dimension: .lifecycle,
                        importance: .key,
                        operationID: operationID
                    )
                    break
                }
                cachedSnapshot = CloudCatalogSnapshot(
                    scope: snapshot.scope,
                    games: snapshot.games,
                    baseUpdatedAt: snapshot.baseUpdatedAt,
                    overlayUpdatedAt: snapshot.overlayUpdatedAt,
                    successfulImageURLs: cachedSnapshot?.successfulImageURLs
                        ?? snapshot.successfulImageURLs,
                    unavailableSources: snapshot.unavailableSources ?? [],
                    sourceUpdatedAt: snapshot.sourceUpdatedAt
                )
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
                        "elapsedMs": .integer(
                            Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: pageStartedAt))
                        ),
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
                        "elapsedMs": .integer(
                            Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: pageStartedAt))
                        ),
                    ],
                    dimension: .lifecycle,
                    importance: .key,
                    operationID: operationID
                )
                break
            } catch {
                failedPages += 1
                if isCurrent(revision) {
                    metadataState.failure = SectionRequestFailure(error: error)
                }
                IOSRuntimeTrace.event(
                    domain: "cloud-library",
                    event: "metadataPageFailed",
                    payload: CloudLibraryDiagnostics.errorPayload(
                        error,
                        extra: [
                            "pageIndex": .integer(Int64(pageIndex)),
                            "pageCount": .integer(Int64(pages.count)),
                            "requested": .integer(Int64(page.count)),
                            "elapsedMs": .integer(
                                Int64(
                                    CloudLibraryDiagnostics.elapsedMilliseconds(
                                        since: pageStartedAt))),
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
                achievementProgress: game.achievementProgress,
                recentRank: game.recentRank,
                newRank: game.newRank, popularRank: game.popularRank, leavingRank: game.leavingRank
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
                achievementProgress: activity.achievementProgress,
                recentRank: game.recentRank,
                newRank: game.newRank, popularRank: game.popularRank, leavingRank: game.leavingRank
            )
        }
    }

    private func scope(accountID: String, regionHost: String) -> CloudCatalogScope {
        CloudCatalogScope(
            accountID: accountID,
            regionHost: regionHost,
            language: language,
            market: market
        )
    }

    nonisolated static func catalogLanguage(preferredLanguage: String?) -> String {
        let normalized =
            preferredLanguage?
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased() ?? ""
        return normalized.hasPrefix("zh") ? "zh-TW" : "en-US"
    }

    nonisolated static func catalogMarket() -> String {
        "US"
    }

    private var language: String {
        // 与桌面 XCloud 页保持同一目录查询合同，避免设备 Locale 直接改变目录集合。
        settingsProvider?.mobile.catalogLanguage
            ?? Self.catalogLanguage(preferredLanguage: Locale.preferredLanguages.first)
    }

    private var market: String {
        settingsProvider?.mobile.catalogMarket ?? Self.catalogMarket()
    }
}

private struct RemoteLoadResult: Sendable {
    let access: PreparedCloudAccess
    let snapshot: RemoteCloudCatalogSnapshot
}

extension Array {
    fileprivate func chunked(maxCount: Int) -> [[Element]] {
        guard maxCount > 0 else {
            return []
        }
        return stride(from: 0, to: count, by: maxCount).map { start in
            Array(self[start..<Swift.min(start + maxCount, count)])
        }
    }
}

extension CloudLibraryStore {
    var catalogSettingsKey: String { "\(market):\(language)" }
    var productScopeID: String { "\(productQueryGeneration):\(catalogSettingsKey)" }

    func productDetailState(_ productID: String) -> XboxSingleDataState<CloudProductDetails> {
        productDetails["\(catalogSettingsKey):\(productID)"] ?? XboxSingleDataState()
    }

    func waitTimeState(productID: String) -> XboxSingleDataState<UInt64> {
        let region = settingsProvider?.streamingSessionSettings.cloudServerRegion ?? "auto"
        return waitTimes["wait:\(productID):\(region)"] ?? XboxSingleDataState()
    }

    func loadProductDetail(productID: String, refresh: Bool = false) async {
        let identity = "\(catalogSettingsKey):\(productID)"
        var state = productDetails[identity] ?? XboxSingleDataState()
        guard productRequestIDs[identity] == nil,
            refresh
                || (state.updatedAt.map({ Date().timeIntervalSince($0) > 600 })
                    ?? (state.phase == .idle))
        else { return }
        let scopeID = productScopeID
        let request = UUID()
        productRequestIDs[identity] = request
        defer {
            if productRequestIDs[identity] == request {
                productRequestIDs[identity] = nil
                if productScopeID != scopeID { productDetails[identity] = nil }
            }
        }
        state.beginRequest()
        productDetails[identity] = state
        do {
            let detail = try await client.loadProductDetail(
                productID: productID, market: market, language: language)
            guard productScopeID == scopeID, productRequestIDs[identity] == request else { return }
            guard detail.productID.caseInsensitiveCompare(productID) == .orderedSame else {
                throw XboxNativeQueryError.unavailable
            }
            state.value = detail
            state.phase = .loaded
            state.updatedAt = .now
        } catch {
            guard productScopeID == scopeID, productRequestIDs[identity] == request else { return }
            state.failRequest(error)
            if state.failure?.kind == .authentication { authentication?.requireDataSignIn() }
        }
        state.isRefreshing = false
        productDetails[identity] = state
        productRequestIDs[identity] = nil
    }

    func loadWaitTime(
        productID: String, streamTitleID: String, refresh: Bool = false,
        prepareAccess: @escaping @MainActor () async throws -> PreparedCloudAccess
    ) async {
        let region = settingsProvider?.streamingSessionSettings.cloudServerRegion ?? "auto"
        let key = "wait:\(productID):\(region)"
        var state = waitTimes[key] ?? XboxSingleDataState()
        guard productRequestIDs[key] == nil,
            refresh
                || (state.updatedAt.map({ Date().timeIntervalSince($0) > 60 })
                    ?? (state.phase == .idle))
        else { return }
        let scopeID = productScopeID
        let request = UUID()
        productRequestIDs[key] = request
        defer {
            if productRequestIDs[key] == request {
                productRequestIDs[key] = nil
                if productScopeID != scopeID { waitTimes[key] = nil }
            }
        }
        state.beginRequest()
        waitTimes[key] = state
        do {
            if activeAccess == nil
                || (activeAccess?.expiresAtMs ?? 0) <= UInt64(Date().timeIntervalSince1970 * 1000)
            {
                let access = try await prepareAccess()
                guard productScopeID == scopeID, productRequestIDs[key] == request else { return }
                activeAccess = access
            }
        } catch {
            guard productScopeID == scopeID, productRequestIDs[key] == request else { return }
            state.failRequest(error)
            if state.failure?.kind == .authentication { authentication?.requireDataSignIn() }
            waitTimes[key] = state
            return
        }
        guard let access = activeAccess else { return }
        do {
            let seconds = try await client.loadWaitTime(
                accessHandle: access.handle, streamTitleID: streamTitleID, serverRegion: region)
            guard productScopeID == scopeID, productRequestIDs[key] == request else { return }
            state.value = seconds
            state.phase = .loaded
            state.updatedAt = .now
        } catch {
            guard productScopeID == scopeID, productRequestIDs[key] == request else { return }
            state.failRequest(error)
            if state.failure?.kind == .authentication { authentication?.requireDataSignIn() }
        }
        state.isRefreshing = false
        waitTimes[key] = state
        productRequestIDs[key] = nil
    }

    func retrySource(
        _ source: String, prepareAccess: @escaping @MainActor () async throws -> PreparedCloudAccess
    ) async {
        let key = "catalog-source-\(source)"
        guard productRequestIDs[key] == nil, !isRefreshing, refreshTask == nil else { return }
        let revision = generation
        let scope = activeScope
        let request = UUID()
        productRequestIDs[key] = request
        var state = sourceStates[source] ?? XboxSingleDataState<Bool>()
        state.beginRequest()
        sourceStates[source] = state
        defer { if productRequestIDs[key] == request { productRequestIDs[key] = nil } }
        do {
            var access: PreparedCloudAccess
            if let saved = activeAccess,
                saved.expiresAtMs > UInt64(Date().timeIntervalSince1970 * 1000)
            {
                access = saved
            } else {
                access = try await prepareAccess()
            }
            let ids: [String]
            do {
                ids = try await client.loadCatalogSource(
                    accessHandle: access.handle, source: source, market: market, language: language)
            } catch {
                guard SectionRequestFailure(error: error).kind == .authentication else {
                    throw error
                }
                access = try await prepareAccess()
                ids = try await client.loadCatalogSource(
                    accessHandle: access.handle, source: source, market: market, language: language)
            }
            try Task.checkCancellation()
            guard generation == revision, activeScope == scope, productRequestIDs[key] == request
            else {
                return
            }
            activeAccess = access
            try await commitSourceRanks(
                ids, source: source, revision: revision, scope: scope, key: key, request: request)
            state.value = true
            state.phase = .loaded
            state.updatedAt = .now
            state.failure = nil
            state.errorMessage = nil
        } catch {
            guard generation == revision, activeScope == scope, productRequestIDs[key] == request
            else {
                return
            }
            state.failRequest(error)
            if state.failure?.kind == .authentication { authentication?.requireDataSignIn() }
        }
        state.isRefreshing = false
        sourceStates[source] = state
    }

    /// 来源请求可并行，快照合并顺序提交，保留其他来源刚取得的结果。
    private func commitSourceRanks(
        _ ids: [String], source: String, revision: Int, scope: CloudCatalogScope?,
        key: String, request: UUID
    ) async throws {
        let previous = sourceCommitTask
        let commitID = UUID()
        let commit = Task { @MainActor [weak self] in
            _ = try? await previous?.value
            try Task.checkCancellation()
            guard let self, self.generation == revision, self.activeScope == scope,
                self.productRequestIDs[key] == request
            else { throw CancellationError() }
            try await self.applySourceRanks(
                ids, source: source, revision: revision, scope: scope, key: key, request: request)
        }
        sourceCommitTask = commit
        sourceCommitTaskID = commitID
        defer {
            if sourceCommitTaskID == commitID {
                sourceCommitTask = nil
                sourceCommitTaskID = nil
            }
        }
        try await withTaskCancellationHandler {
            try await commit.value
        } onCancel: {
            commit.cancel()
        }
    }

    private func applySourceRanks(
        _ ids: [String], source: String, revision: Int, scope: CloudCatalogScope?,
        key: String, request: UUID
    ) async throws {
        let ranks = Dictionary(
            ids.enumerated().map { ($0.element.uppercased(), $0.offset) },
            uniquingKeysWith: { first, _ in first })
        let updated = games.map { game in
            var item = game
            let rank = ranks[game.productID.uppercased()]
            switch source {
            case "recent":
                item.recentRank = rank
                item.isRecentlyPlayed = rank != nil
            case "newest":
                item.newRank = rank
                item.isNew = rank != nil
            case "popular": item.popularRank = rank
            case "leaving": item.leavingRank = rank
            default: break
            }
            return item
        }
        var unavailable = unavailableSources
        unavailable.remove(source)
        if let saved = cachedSnapshot {
            let snapshot = CloudCatalogSnapshot(
                scope: saved.scope, games: updated,
                baseUpdatedAt: saved.baseUpdatedAt, overlayUpdatedAt: .now,
                successfulImageURLs: saved.successfulImageURLs, unavailableSources: unavailable,
                sourceUpdatedAt: (saved.sourceUpdatedAt ?? [:]).merging(
                    [source: .now], uniquingKeysWith: { _, latest in latest }))
            try await repository.save(snapshot)
            guard generation == revision, activeScope == scope,
                productRequestIDs[key] == request, !Task.isCancelled
            else {
                await discardPersistedOverlayIfOwnerChanged(scope: snapshot.scope)
                throw CancellationError()
            }
            cachedSnapshot = snapshot
        }
        try Task.checkCancellation()
        games = updated
        unavailableSources = unavailable
    }

    func retryMetadata(prepareAccess: @escaping @MainActor () async throws -> PreparedCloudAccess)
        async
    {
        guard !metadataState.isRefreshing, !isRefreshing else { return }
        guard let scope = activeScope, let access = activeAccess, !pendingMetadataIDs.isEmpty else {
            await refresh(reason: .manualRetry, prepareAccess: prepareAccess)
            return
        }
        let revision = generation
        metadataState.beginRequest()
        let failed = await hydrateMetadataPages(
            productIDs: Array(pendingMetadataIDs), access: access,
            scope: scope, fetchedAt: .now, requestGeneration: revision,
            operationID: UUID().uuidString)
        guard revision == generation else { return }
        metadataState.isRefreshing = false
        failedHydrationChunks = failed
        if Task.isCancelled {
            metadataState.failRequest(CancellationError())
        } else if failed == 0 {
            metadataState.value = true
            metadataState.phase = .loaded
            metadataState.updatedAt = .now
            metadataState.failure = nil
        } else {
            metadataState.phase = metadataState.hasSnapshot ? .loaded : .failed
        }
    }

    private func preserveDiscoveryRanks(_ incoming: [CloudLibraryGame], unavailable: Set<String>)
        -> [CloudLibraryGame]
    {
        let previous = Dictionary(uniqueKeysWithValues: games.map { ($0.productID, $0) })
        return incoming.map { game in
            var updated = game
            guard let saved = previous[game.productID] else { return updated }
            if unavailable.contains("recent") {
                updated.recentRank = saved.recentRank
                updated.isRecentlyPlayed = saved.isRecentlyPlayed
            }
            if unavailable.contains("newest") {
                updated.newRank = saved.newRank
                updated.isNew = saved.isNew
            }
            if unavailable.contains("popular") { updated.popularRank = saved.popularRank }
            if unavailable.contains("leaving") { updated.leavingRank = saved.leavingRank }
            return updated
        }
    }
}
