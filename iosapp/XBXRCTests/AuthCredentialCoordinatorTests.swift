import XCTest
@testable import XBXRC

final class AuthCredentialCoordinatorTests: XCTestCase {
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

    private static let storedSession = StoredAuthSession(
        refreshToken: "stored-refresh",
        seedJSON: "seed",
        webTokenJSON: "stored-web",
        appLevel: 1,
        cloudAccountID: "account",
        cloudRegionHost: "cloud.example.com"
    )
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

    init(session: StoredAuthSession? = nil, deleteFailures: Int = 0) {
        self.session = session
        remainingDeleteFailures = deleteFailures
    }

    func load() async throws -> StoredAuthSession? {
        session
    }

    func save(_ session: StoredAuthSession) async throws {
        self.session = session
    }

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
        refreshToken _: String,
        seedJSON _: String,
        forceRegionIP _: String
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
    private var releases: [String] = []

    init(prepareGate: AuthTestGate? = nil) {
        self.prepareGate = prepareGate
    }

    func prepareAccess(
        refreshToken _: String,
        seedJSON _: String,
        forceRegionIP _: String
    ) async throws -> PreparedCloudAccess {
        await prepareGate?.wait()
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

    func prepareHomeAccess(
        refreshToken _: String,
        seedJSON _: String,
        forceRegionIP _: String
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
