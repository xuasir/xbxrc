import Synchronization
import XCTest
@testable import XBXRC

final class AuthCredentialCoordinatorTests: XCTestCase {
    @MainActor
    func testRealBridgeReusesWebHomeAndCloudCacheBeforeParsingSeed() async throws {
        let session = try Self.cachedBridgeSession()
        let initialHandles = uniffiForeignFutureHandleCountXboxIosBridge()
        for scope in [XboxAuthScope.web, .home, .cloud] {
            let observer = AuthBridgeTestObserver()
            let output = try await refreshLogin(
                session: session, forceRegionIp: "", scope: scope,
                forceCoreRefresh: false, rejectedToken: nil, observer: observer)
            XCTAssertEqual(output, session)
            XCTAssertEqual(observer.snapshot.stages.map(\.stage), ["validCredentialsReused"])
            XCTAssertTrue(observer.snapshot.checkpoints.isEmpty)
        }
        XCTAssertEqual(uniffiForeignFutureHandleCountXboxIosBridge(), initialHandles)
    }

    @MainActor
    func testRealBridgeAwaitsCoreRecoveryCheckpointAndReleasesForeignFuture() async throws {
        let session = try Self.cachedBridgeSession()
        let observer = AuthBridgeTestObserver()
        let initialHandles = uniffiForeignFutureHandleCountXboxIosBridge()
        do {
            _ = try await refreshLogin(
                session: session, forceRegionIp: "", scope: .home,
                forceCoreRefresh: true, rejectedToken: nil, observer: observer)
            XCTFail("Invalid seed must stop before an authorization request")
        } catch let error as XboxBridgeError {
            guard case .InvalidData = error else { return XCTFail("Expected invalid seed data") }
        }
        let checkpoints = observer.snapshot.checkpoints
        XCTAssertEqual(checkpoints.count, 1)
        let update = try XCTUnwrap(checkpoints.first)
        XCTAssertTrue(update.coreOnly)
        XCTAssertEqual(update.previousRefreshToken, session.refreshToken)
        let cacheData = try XCTUnwrap(update.authSession.tokenCacheJson?.data(using: .utf8))
        let cache = try XCTUnwrap(JSONSerialization.jsonObject(with: cacheData) as? [String: Any])
        XCTAssertEqual(cache["core_refresh_scopes"] as? [String], ["home"])
        XCTAssertTrue(observer.snapshot.stages.isEmpty)
        XCTAssertEqual(uniffiForeignFutureHandleCountXboxIosBridge(), initialHandles)
    }

    @MainActor
    func testRealBridgeReturnsCheckpointFailureBeforeCredentialRequests() async throws {
        let observer = AuthBridgeTestObserver(failsCheckpoint: true)
        let initialHandles = uniffiForeignFutureHandleCountXboxIosBridge()
        do {
            _ = try await refreshLogin(
                session: Self.cachedBridgeSession(), forceRegionIp: "", scope: .cloud,
                forceCoreRefresh: true, rejectedToken: nil, observer: observer)
            XCTFail("Checkpoint failure must stop credential preparation")
        } catch let error as XboxBridgeError {
            guard case .InvalidData(let reason) = error else {
                return XCTFail("Expected the foreign checkpoint error")
            }
            XCTAssertEqual(reason, "fixtureCheckpointRejected")
        }
        XCTAssertEqual(observer.snapshot.checkpoints.count, 1)
        XCTAssertTrue(observer.snapshot.stages.isEmpty)
        XCTAssertEqual(uniffiForeignFutureHandleCountXboxIosBridge(), initialHandles)
    }

    private static func cachedBridgeSession() throws -> AuthSession {
        let now = Date()
        let web: [String: Any] = [
            "data": [
                "Token": "cached-web", "NotAfter": now.addingTimeInterval(3600).ISO8601Format(),
                "DisplayClaims": ["xui": [["xid": "fixture-account", "uhs": "fixture-uhs"]]],
            ]
        ]
        func stream(_ token: String) -> [String: Any] {
            [
                "_objectCreateTime": Int64(now.timeIntervalSince1970 * 1000),
                "data": ["gsToken": token, "durationInSeconds": 3600],
            ]
        }
        let cache: [String: Any] = [
            "schema_version": 1, "force_core_refresh": false, "cloud_force_region_ip": "",
            "stream_tokens": [
                "xHomeToken": stream("cached-home"), "xCloudToken": stream("cached-cloud"),
            ],
        ]
        func json(_ object: [String: Any]) throws -> String {
            String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
        }
        // 热路径成功证明无需解析 seed；强制恢复通过同一 fixture 在网络请求前结束。
        return try AuthSession(
            refreshToken: "cached-refresh", seedJson: "invalid-seed", webTokenJson: json(web),
            appLevel: 2, tokenCacheJson: json(cache))
    }

    func testLegacyKeychainSessionDecodesWithOptionalTokenCache() throws {
        let data = Data(
            #"{"refreshToken":"legacy","seedJSON":"seed","webTokenJSON":"web","appLevel":1}"#.utf8)
        let session = try JSONDecoder().decode(StoredAuthSession.self, from: data)
        XCTAssertNil(session.tokenCacheJSON)
        XCTAssertEqual(session.bridgeSession.refreshToken, "legacy")
        XCTAssertEqual(
            try JSONDecoder().decode(StoredAuthSession.self, from: JSONEncoder().encode(session)),
            session)
    }

    @MainActor
    func testCancelledQueuedMutationSkipsCredentialWork() async throws {
        let coordinator = AuthCredentialCoordinator()
        let gate = AuthTestGate()
        let events = AuthTestEventRecorder()
        let first = Task { @MainActor in
            try await coordinator.perform(advancesRevision: true) { _ in
                await gate.wait(); return 1
            }
        }
        await gate.waitUntilEntered()
        let queued = Task { @MainActor in
            try await coordinator.perform(advancesRevision: true) { _ in
                await events.append("cancelled-work"); return 2
            }
        }
        await Task.yield()
        queued.cancel()
        await gate.open()
        _ = try await first.value
        do { _ = try await queued.value; XCTFail("Cancelled mutation must be skipped") } catch {
            XCTAssertTrue(AuthStoreError.isCancelled(error))
        }
        let recorded = await events.snapshot()
        XCTAssertTrue(recorded.isEmpty)
        XCTAssertEqual(coordinator.currentRevision, 1)
    }

    @MainActor
    func testCoreCheckpointSurvivesDownstreamTimeout() async throws {
        let keychain = AuthTestSessionStore(session: Self.storedSession)
        let cloud = AuthTestCloudClient(checkpointBeforeResult: true, failsAfterCheckpoint: true)
        let store = makeStore(keychain: keychain, cloudClient: cloud)
        await store.restore()
        do {
            _ = try await store.prepareCloudAccess(); XCTFail("Expected downstream timeout")
        } catch { XCTAssertEqual(SectionRequestFailure(error: error).kind, .timeout) }
        XCTAssertEqual(store.session?.refreshToken, "checkpoint-refresh")
        XCTAssertEqual(store.session?.tokenCacheJSON, "checkpoint-cache")
        let persisted = await keychain.currentSession()
        XCTAssertEqual(persisted, store.session)
        XCTAssertEqual(store.phase, .signedIn)
    }

    @MainActor
    func testCheckpointSaveFailureStopsAccessPreparation() async throws {
        let keychain = AuthTestSessionStore(session: Self.storedSession)
        let cloud = AuthTestCloudClient(checkpointBeforeResult: true)
        let store = makeStore(keychain: keychain, cloudClient: cloud)
        await store.restore()
        await keychain.failNextSave()
        do {
            _ = try await store.prepareCloudAccess(); XCTFail("Expected checkpoint storage failure")
        } catch {}
        XCTAssertEqual(store.session?.refreshToken, "renewed-refresh")
        let completed = await cloud.completedPreparations()
        XCTAssertEqual(completed, 0)
    }

    @MainActor
    func testCancellationAfterCoreCheckpointPreservesRotationAndReleasesAccess() async throws {
        let keychain = AuthTestSessionStore(session: Self.storedSession)
        let gate = AuthTestGate()
        let cloud = AuthTestCloudClient(prepareGate: gate, checkpointBeforeResult: true)
        let store = makeStore(keychain: keychain, cloudClient: cloud)
        await store.restore()
        let access = Task { @MainActor in try await store.prepareCloudAccess() }
        await gate.waitUntilEntered()
        access.cancel()
        await gate.open()
        do {
            _ = try await access.value; XCTFail("Cancelled preparation must release access")
        } catch { XCTAssertTrue(AuthStoreError.isCancelled(error)) }
        XCTAssertEqual(store.session?.refreshToken, "checkpoint-refresh")
        let persisted = await keychain.currentSession()
        let released = await cloud.releasedHandles()
        XCTAssertEqual(persisted, store.session)
        XCTAssertEqual(released, ["delayed-access"])
    }
    @MainActor
    func testCoordinatorSerializesCredentialMutations() async throws {
        let coordinator = AuthCredentialCoordinator()
        let gate = AuthTestGate()
        let events = AuthTestEventRecorder()

        let first = Task { @MainActor in
            try await coordinator.perform(advancesRevision: true) { revision in
                await events.append("first-start-\(revision)")
                await gate.wait()
                await events.append("first-end")
                return revision
            }
        }
        await gate.waitUntilEntered()

        let second = Task { @MainActor in
            try await coordinator.perform(advancesRevision: true) { revision in
                await events.append("second-start-\(revision)")
                return revision
            }
        }
        await Task.yield()
        let eventsWhileFirstIsSuspended = await events.snapshot()
        XCTAssertEqual(eventsWhileFirstIsSuspended, ["first-start-0"])

        await gate.open()
        let firstCommit = try await first.value
        let secondCommit = try await second.value

        XCTAssertEqual(firstCommit.value, 0)
        XCTAssertEqual(firstCommit.revision, 1)
        XCTAssertEqual(secondCommit.value, 1)
        XCTAssertEqual(secondCommit.revision, 2)
        let completedEvents = await events.snapshot()
        XCTAssertEqual(
            completedEvents,
            ["first-start-0", "first-end", "second-start-1"]
        )
    }

    @MainActor
    func testCoordinatorRejectsMutationInvalidatedDuringAwait() async {
        let coordinator = AuthCredentialCoordinator()
        let gate = AuthTestGate()
        let task = Task { @MainActor in
            try await coordinator.perform(
                expectedRevision: 0,
                advancesRevision: true
            ) { _ in
                await gate.wait()
                return "completed"
            }
        }
        await gate.waitUntilEntered()

        XCTAssertEqual(coordinator.invalidate(), 1)
        await gate.open()

        do {
            _ = try await task.value
            XCTFail("失效的 credential mutation 不应提交")
        } catch {
            XCTAssertTrue(AuthStoreError.isStale(error))
        }
        XCTAssertEqual(coordinator.currentRevision, 1)
    }

    @MainActor
    func testRenewalFailureClassifiesTransientTransportErrorsAsRetryable() {
        XCTAssertEqual(
            AuthRenewalFailureDisposition(error: URLError(.timedOut)),
            .retryable
        )
        XCTAssertEqual(
            AuthRenewalFailureDisposition(
                error: XboxBridgeError.RetryableAuthentication("Network error: connection reset")
            ),
            .retryable
        )
        XCTAssertEqual(
            AuthRenewalFailureDisposition(
                error: XboxBridgeError.RetryableAuthentication("HTTP 503: unavailable")
            ),
            .retryable
        )
    }

    @MainActor
    func testRenewalFailureClassifiesCredentialAndPersistenceErrorsAsTerminal() {
        XCTAssertEqual(
            AuthRenewalFailureDisposition(
                error: XboxBridgeError.Authentication("Auth error: invalid_grant")
            ),
            .terminal
        )
        XCTAssertEqual(
            AuthRenewalFailureDisposition(
                error: XboxBridgeError.InvalidData("invalid seed")
            ),
            .terminal
        )
        XCTAssertEqual(
            AuthRenewalFailureDisposition(error: AuthTestError.deleteFailed),
            .terminal
        )
    }

    @MainActor
    func testSignOutRetainsSessionWhenKeychainDeleteFailsAndAllowsRetry() async {
        let keychain = AuthTestSessionStore(
            session: Self.storedSession,
            deleteFailures: 1
        )
        let store = makeStore(keychain: keychain)
        await store.restore()

        await store.signOut()

        let retainedKeychainSession = await keychain.currentSession()
        let firstDeleteCount = await keychain.deleteCount()
        XCTAssertEqual(store.phase, .failed)
        XCTAssertEqual(store.session?.refreshToken, "renewed-refresh")
        XCTAssertNotNil(retainedKeychainSession)
        XCTAssertEqual(firstDeleteCount, 1)

        await store.retry()

        let deletedKeychainSession = await keychain.currentSession()
        let finalDeleteCount = await keychain.deleteCount()
        XCTAssertEqual(store.phase, .signedOut)
        XCTAssertNil(store.session)
        XCTAssertNil(deletedKeychainSession)
        XCTAssertEqual(finalDeleteCount, 2)
    }

    @MainActor
    func testConcurrentCloudQueriesShareOnePreparedLease() async throws {
        let keychain = AuthTestSessionStore(session: Self.storedSession)
        let gate = AuthTestGate()
        let cloud = AuthTestCloudClient(prepareGate: gate)
        let store = makeStore(keychain: keychain, cloudClient: cloud)
        await store.restore()

        let first = Task { @MainActor in try await store.prepareCloudAccess() }
        await gate.waitUntilEntered()
        let secondStarted = expectation(description: "second cloud query started")
        let second = Task { @MainActor in
            secondStarted.fulfill()
            return try await store.prepareCloudAccess()
        }
        await fulfillment(of: [secondStarted], timeout: 1)
        await gate.open()

        let firstAccess = try await first.value
        let secondAccess = try await second.value
        let preparations = await cloud.completedPreparations()
        XCTAssertEqual(preparations, 1)
        XCTAssertEqual(firstAccess.handle, secondAccess.handle)
        XCTAssertEqual(firstAccess.ownerGeneration, secondAccess.ownerGeneration)
    }

    @MainActor
    func testCancellingOneCloudWaiterKeepsOtherQueryLease() async throws {
        let keychain = AuthTestSessionStore(session: Self.storedSession)
        let gate = AuthTestGate()
        let cloud = AuthTestCloudClient(prepareGate: gate)
        let store = makeStore(keychain: keychain, cloudClient: cloud)
        await store.restore()

        let cancelled = expectation(description: "cancelled query released its waiter")
        let first = Task { @MainActor in
            do {
                _ = try await store.prepareCloudAccess()
                XCTFail("Cancelled query must finish with cancellation")
            } catch { XCTAssertTrue(SectionRequestFailure.isCancellation(error)) }
            cancelled.fulfill()
        }
        await gate.waitUntilEntered()
        let secondStarted = expectation(description: "remaining cloud query started")
        let second = Task { @MainActor in
            secondStarted.fulfill()
            return try await store.prepareCloudAccess()
        }
        await fulfillment(of: [secondStarted], timeout: 1)
        first.cancel()
        await fulfillment(of: [cancelled], timeout: 1)
        await gate.open()

        await first.value
        let access = try await second.value
        let preparations = await cloud.completedPreparations()
        let releases = await cloud.releasedHandles()
        XCTAssertEqual(access.handle, "delayed-access")
        XCTAssertEqual(preparations, 1)
        XCTAssertTrue(releases.isEmpty)
    }

    @MainActor
    func testSignOutInvalidatesDelayedAccessAndReleasesItsHandle() async {
        let keychain = AuthTestSessionStore(session: Self.storedSession)
        let accessGate = AuthTestGate()
        let cloudClient = AuthTestCloudClient(prepareGate: accessGate)
        let store = makeStore(keychain: keychain, cloudClient: cloudClient)
        await store.restore()

        let accessTask = Task { @MainActor in
            try await store.prepareCloudAccess()
        }
        await accessGate.waitUntilEntered()

        let signOutTask = Task { @MainActor in
            await store.signOut()
        }
        for _ in 0 ..< 100 where store.phase != .refreshing {
            await Task.yield()
        }
        XCTAssertEqual(store.phase, .refreshing)
        await accessGate.open()

        do {
            _ = try await accessTask.value
            XCTFail("退出后的 access 结果不应交给调用方")
        } catch {
            XCTAssertTrue(AuthStoreError.isStale(error))
        }
        await signOutTask.value

        let releasedHandles = await cloudClient.releasedHandles()
        let keychainSession = await keychain.currentSession()
        XCTAssertEqual(releasedHandles, ["delayed-access"])
        XCTAssertEqual(store.phase, .signedOut)
        XCTAssertNil(store.session)
        XCTAssertNil(keychainSession)
    }

    @MainActor
    private func makeStore(
        keychain: AuthTestSessionStore,
        cloudClient: AuthTestCloudClient = AuthTestCloudClient()
    ) -> AuthStore {
        AuthStore(
            settings: AuthTestRegionSettings(),
            client: AuthTestXboxAuthClient(),
            cloudClient: cloudClient,
            keychain: keychain,
            webAuthentication: AuthTestWebAuthentication()
        )
    }

    @MainActor
    func testStreamingRenewalPersistsAndRetriesKeychainFailure() async throws {
        let keychain = AuthTestSessionStore(session: Self.storedSession)
        let store = makeStore(keychain: keychain)
        await store.restore()
        let update = Self.streamingUpdate()
        await keychain.failNextSave()
        do {
            _ = try await store.persistStreamingAuthUpdate(update, expectedOwnerGeneration: store.ownerGeneration)
            XCTFail("Keychain failure should retain the pending credential update")
        } catch {}
        XCTAssertEqual(store.session?.refreshToken, "renewed-refresh")
        let result = try await store.persistStreamingAuthUpdate(update, expectedOwnerGeneration: store.ownerGeneration)
        guard case .committed = result else { return XCTFail("Expected persisted credentials") }
        let stored = await keychain.currentSession()
        XCTAssertEqual(stored?.refreshToken, "stream-refresh")
        XCTAssertEqual(store.session?.webTokenJSON, "stream-web")
        XCTAssertEqual(store.session?.cloudRegionHost, "cloud.example.com")
    }

    @MainActor
    func testStreamingRenewalSynchronizesNewerHostCredentialsAndRejectsWrongOwner() async throws {
        let keychain = AuthTestSessionStore(session: Self.storedSession)
        let store = makeStore(keychain: keychain)
        await store.restore()
        let update = Self.streamingUpdate(previous: "older-refresh")
        let result = try await store.persistStreamingAuthUpdate(update, expectedOwnerGeneration: store.ownerGeneration)
        guard case .synchronize(let session) = result else { return XCTFail("Expected current host credentials") }
        XCTAssertEqual(session.refreshToken, "renewed-refresh")
        XCTAssertEqual(store.session?.refreshToken, "renewed-refresh")
        for (account, owner) in [("other-account", store.ownerGeneration), ("account", store.ownerGeneration + 1)] {
            do {
                _ = try await store.persistStreamingAuthUpdate(Self.streamingUpdate(account: account), expectedOwnerGeneration: owner)
                XCTFail("Credential owner should be checked before persistence")
            } catch { XCTAssertTrue(AuthStoreError.isStale(error)) }
        }
    }

    @MainActor
    func testSignOutInvalidatesStreamingCredentialSaveAndDeletesItsResult() async throws {
        let keychain = AuthTestSessionStore(session: Self.storedSession)
        let store = makeStore(keychain: keychain)
        await store.restore()
        let owner = store.ownerGeneration
        let gate = AuthTestGate()
        await keychain.delayNextSave(gate)
        let save = Task { @MainActor in
            try await store.persistStreamingAuthUpdate(Self.streamingUpdate(), expectedOwnerGeneration: owner)
        }
        await gate.waitUntilEntered()
        let signOut = Task { @MainActor in await store.signOut() }
        for _ in 0 ..< 100 where store.ownerGeneration == owner { await Task.yield() }
        XCTAssertNotEqual(store.ownerGeneration, owner)
        await gate.open()
        do { _ = try await save.value; XCTFail("Revoked renewal should discard its commit") }
        catch { XCTAssertTrue(AuthStoreError.isStale(error)) }
        await signOut.value
        let stored = await keychain.currentSession()
        XCTAssertNil(stored)
        XCTAssertNil(store.session)
    }

    private static func streamingUpdate(previous: String = "renewed-refresh", account: String = "account") -> XboxStreamingAuthUpdate {
        XboxStreamingAuthUpdate(revision: 1, accountId: account, ownerGeneration: 1,
            previousRefreshToken: previous, authSession: AuthSession(refreshToken: "stream-refresh",
                seedJson: "seed", webTokenJson: "stream-web", appLevel: 2))
    }

    @MainActor
    func testSameRefreshTokenStillPersistsUpdatedDownstreamCache() async throws {
        let keychain = AuthTestSessionStore(session: Self.storedSession)
        let store = makeStore(keychain: keychain)
        await store.restore()
        var update = Self.streamingUpdate()
        update.authSession = AuthSession(
            refreshToken: "renewed-refresh", seedJson: "seed",
            webTokenJson: "updated-web", appLevel: 2, tokenCacheJson: "target-cache")
        let result = try await store.persistStreamingAuthUpdate(
            update, expectedOwnerGeneration: store.ownerGeneration)
        guard case .committed = result else { return XCTFail("Expected downstream update commit") }
        XCTAssertEqual(store.session?.refreshToken, "renewed-refresh")
        XCTAssertEqual(store.session?.tokenCacheJSON, "target-cache")
        let persisted = await keychain.currentSession()
        XCTAssertEqual(persisted?.webTokenJSON, "updated-web")
    }

    private static let storedSession = StoredAuthSession(
        refreshToken: "stored-refresh",
        seedJSON: "seed",
        webTokenJSON: "stored-web",
        appLevel: 1,
        cloudAccountID: "account",
        cloudRegionHost: "cloud.example.com"
    )
}

private final class AuthBridgeTestObserver: XboxAuthObserver, Sendable {
    struct Snapshot: Sendable {
        var checkpoints: [XboxAuthCheckpoint] = []
        var stages: [XboxAuthStage] = []
    }
    private let state = Mutex(Snapshot())
    private let failsCheckpoint: Bool
    var snapshot: Snapshot { state.withLock { $0 } }

    init(failsCheckpoint: Bool = false) { self.failsCheckpoint = failsCheckpoint }

    func checkpoint(update: XboxAuthCheckpoint) async throws {
        state.withLock { $0.checkpoints.append(update) }
        if failsCheckpoint { throw XboxBridgeError.InvalidData("fixtureCheckpointRejected") }
    }

    func onStage(event: XboxAuthStage) { state.withLock { $0.stages.append(event) } }
}

private enum AuthTestError: LocalizedError {
    case deleteFailed
    case unexpectedCall

    var errorDescription: String? {
        switch self {
        case .deleteFailed:
            "测试 Keychain 删除失败"
        case .unexpectedCall:
            "测试调用超出预期"
        }
    }
}

private actor AuthTestGate {
    private var entered = false
    private var operationContinuation: CheckedContinuation<Void, Never>?
    private var enteredContinuations: [CheckedContinuation<Void, Never>] = []

    func wait() async {
        entered = true
        enteredContinuations.forEach { $0.resume() }
        enteredContinuations.removeAll()
        await withCheckedContinuation { continuation in
            operationContinuation = continuation
        }
    }

    func waitUntilEntered() async {
        if entered { return }
        await withCheckedContinuation { continuation in
            enteredContinuations.append(continuation)
        }
    }

    func open() {
        operationContinuation?.resume()
        operationContinuation = nil
    }
}

private actor AuthTestEventRecorder {
    private var events: [String] = []

    func append(_ event: String) {
        events.append(event)
    }

    func snapshot() -> [String] {
        events
    }
}

private actor AuthTestSessionStore: AuthSessionStoring {
    private var session: StoredAuthSession?
    private var remainingDeleteFailures: Int
    private var deletes = 0
    private var nextSaveGate: AuthTestGate?
    private var remainingSaveFailures = 0

    init(session: StoredAuthSession? = nil, deleteFailures: Int = 0) {
        self.session = session
        remainingDeleteFailures = deleteFailures
    }

    func load() async throws -> StoredAuthSession? {
        session
    }

    func save(_ session: StoredAuthSession) async throws {
        let gate = nextSaveGate
        nextSaveGate = nil
        await gate?.wait()
        if remainingSaveFailures > 0 {
            remainingSaveFailures -= 1
            throw AuthTestError.deleteFailed
        }
        self.session = session
    }

    func failNextSave() { remainingSaveFailures = 1 }
    func delayNextSave(_ gate: AuthTestGate) { nextSaveGate = gate }

    func delete() async throws {
        deletes += 1
        if remainingDeleteFailures > 0 {
            remainingDeleteFailures -= 1
            throw AuthTestError.deleteFailed
        }
        session = nil
    }

    func currentSession() -> StoredAuthSession? {
        session
    }

    func deleteCount() -> Int {
        deletes
    }
}

private struct AuthTestXboxAuthClient: XboxAuthClient {
    func beginLogin() async throws -> LoginStartResult {
        throw AuthTestError.unexpectedCall
    }

    func finishLogin(
        callbackURL _: URL,
        pendingJSON _: String,
        seedJSON _: String,
        forceRegionIP _: String
    ) async throws -> AuthSession {
        throw AuthTestError.unexpectedCall
    }

    func renewLogin(
        session: StoredAuthSession, forceRegionIP: String, scope: XboxAuthScope,
        forceCoreRefresh: Bool, rejectedToken: String?,
        checkpoint: @escaping AuthCredentialCheckpoint
    ) async throws -> AuthSession {
        AuthSession(
            refreshToken: "renewed-refresh",
            seedJson: "seed",
            webTokenJson: "renewed-web",
            appLevel: 2
        )
    }

    func loadProfile(webTokenJSON _: String) async throws -> XboxProfile {
        throw AuthTestError.unexpectedCall
    }
}

private actor AuthTestCloudClient: XboxCloudDataClient {
    private let prepareGate: AuthTestGate?
    private let checkpointBeforeResult: Bool
    private let failsAfterCheckpoint: Bool
    private var completed = 0
    private var releases: [String] = []

    init(
        prepareGate: AuthTestGate? = nil, checkpointBeforeResult: Bool = false,
        failsAfterCheckpoint: Bool = false
    ) {
        self.prepareGate = prepareGate
        self.checkpointBeforeResult = checkpointBeforeResult
        self.failsAfterCheckpoint = failsAfterCheckpoint
    }

    func prepareAccess(
        session: StoredAuthSession, forceRegionIP: String,
        checkpoint: @escaping AuthCredentialCheckpoint,
        authProvider: (any XboxStreamingAuthProvider)?, startup: XboxStreamStartup?
    ) async throws -> PreparedCloudAccess {
        if checkpointBeforeResult {
            try await checkpoint(
                XboxAuthCheckpoint(
                    previousRefreshToken: session.refreshToken,
                    authSession: AuthSession(
                        refreshToken: "checkpoint-refresh", seedJson: session.seedJSON,
                        webTokenJson: session.webTokenJSON, appLevel: session.appLevel,
                        tokenCacheJson: "checkpoint-cache"),
                    coreOnly: true))
        }
        if failsAfterCheckpoint {
            throw XboxBridgeError.Query(
                failure: XboxQueryFailure(
                    kind: .timeout, statusCode: nil,
                    retriable: true, retryAfterSeconds: nil))
        }
        await prepareGate?.wait()
        completed += 1
        return PreparedCloudAccess(
            authSession: AuthSession(
                refreshToken: "access-refresh",
                seedJson: "seed",
                webTokenJson: "access-web",
                appLevel: 2
            ),
            handle: "delayed-access",
            accountID: "account",
            regionHost: "cloud.example.com"
        )
    }

    func completedPreparations() -> Int { completed }

    func prepareHomeAccess(
        session: StoredAuthSession, forceRegionIP: String,
        checkpoint: @escaping AuthCredentialCheckpoint,
        authProvider: (any XboxStreamingAuthProvider)?, startup: XboxStreamStartup?
    ) async throws -> PreparedHomeAccess {
        throw AuthTestError.unexpectedCall
    }

    func loadCatalog(
        accessHandle _: String,
        market _: String,
        language _: String
    ) async throws -> RemoteCloudCatalogSnapshot {
        throw AuthTestError.unexpectedCall
    }

    func loadMetadataPage(
        accessHandle _: String,
        market _: String,
        language _: String,
        productIDs _: [String]
    ) async throws -> [CloudCatalogMetadata] {
        throw AuthTestError.unexpectedCall
    }

    func releaseAccess(handle: String) async {
        releases.append(handle)
    }

    func releasedHandles() -> [String] {
        releases
    }
}

@MainActor
private final class AuthTestRegionSettings: CloudRegionSettingsProviding {
    let cloudRegionPreset: CloudRegionPreset = .default
    let usesEphemeralLoginSession = false
}

@MainActor
private final class AuthTestWebAuthentication: WebAuthenticating {
    func authenticate(
        authorizationURL _: String,
        prefersEphemeralSession _: Bool
    ) async throws -> URL {
        throw AuthTestError.unexpectedCall
    }

    func cancel() {}
}
