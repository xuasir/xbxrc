import Foundation
import XCTest
@testable import XBXRC

final class CloudLibraryStoreConcurrencyTests: XCTestCase {
    @MainActor
    func testLayoutQueryReusesAccessBeforeCatalogCommit() async {
        let scope = CloudCatalogScope(
            accountID: "account-a", regionHost: "region.example.com", language: "en-US",
            market: "US")
        let repository = DelayedSnapshotRepository()
        let store = CloudLibraryStore(
            client: DelayedSaveCloudClient(scope: scope), repository: repository)
        let refresh = Task { @MainActor in
            await store.refresh(reason: .manualRetry) {
                PreparedCloudAccess(
                    authSession: AuthSession(
                        refreshToken: "refresh", seedJson: "seed", webTokenJson: "web", appLevel: 1),
                    handle: "catalog-access", accountID: scope.accountID,
                    regionHost: scope.regionHost,
                    ownerGeneration: 1,
                    expiresAtMs: UInt64(Date().timeIntervalSince1970 * 1000) + 600_000)
            }
        }
        await repository.waitUntilSaveEntered()
        do {
            let handle = try await store.withCloudAccess(
                prepareAccess: {
                    XCTFail("Layout query should reuse the pending catalog lease")
                    throw CancellationError()
                }, operation: { $0.handle })
            XCTAssertEqual(handle, "catalog-access")
        } catch { XCTFail("Layout query failed before catalog commit: \(error)") }
        await repository.completeSave()
        await refresh.value
    }

    @MainActor
    func testClearWaitsForDelayedSaveAndRemovesStaleOverlay() async {
        let scope = CloudCatalogScope(
            accountID: "account-a",
            regionHost: "region.example.com",
            language: "en-US",
            market: "US"
        )
        let repository = DelayedSnapshotRepository()
        let client = DelayedSaveCloudClient(scope: scope)
        let store = CloudLibraryStore(client: client, repository: repository)

        let refreshTask = Task { @MainActor in
            await store.refresh(reason: .manualRetry) {
                PreparedCloudAccess(
                    authSession: AuthSession(
                        refreshToken: "refresh",
                        seedJson: "seed",
                        webTokenJson: "web",
                        appLevel: 1
                    ),
                    handle: "access",
                    accountID: scope.accountID,
                    regionHost: scope.regionHost
                )
            }
        }
        await repository.waitUntilSaveEntered()

        let clearTask = Task { @MainActor in
            await store.clear()
        }
        await Task.yield()
        let eventsBeforeSaveCompletes = await repository.events()
        XCTAssertEqual(eventsBeforeSaveCompletes, ["saveStarted"])

        await repository.completeSave()
        await refreshTask.value
        await clearTask.value

        let events = await repository.events()
        let overlayPresent = await repository.overlayPresent()
        XCTAssertEqual(events, ["saveStarted", "saveCompleted", "overlayCleared"])
        XCTAssertFalse(overlayPresent)
        XCTAssertTrue(store.games.isEmpty)
        XCTAssertEqual(store.phase, .idle)
    }
}

private actor DelayedSnapshotRepository: CloudCatalogSnapshotStoring {
    private var saveEntered = false
    private var saveWaiters: [CheckedContinuation<Void, Never>] = []
    private var saveContinuation: CheckedContinuation<Void, Never>?
    private var recordedEvents: [String] = []
    private var hasOverlay = false

    func load(scope _: CloudCatalogScope) async throws -> CloudCatalogSnapshot? { nil }

    func save(_ snapshot: CloudCatalogSnapshot) async throws {
        recordedEvents.append("saveStarted")
        saveEntered = true
        saveWaiters.forEach { $0.resume() }
        saveWaiters.removeAll()
        await withCheckedContinuation { continuation in
            saveContinuation = continuation
        }
        hasOverlay = true
        recordedEvents.append("saveCompleted")
    }

    func saveSuccessfulImagePreferences(_: CloudCatalogSnapshot) async throws {}

    func clearOverlay(accountID _: String) async throws {
        hasOverlay = false
        recordedEvents.append("overlayCleared")
    }

    func waitUntilSaveEntered() async {
        if saveEntered { return }
        await withCheckedContinuation { continuation in
            saveWaiters.append(continuation)
        }
    }

    func completeSave() {
        saveContinuation?.resume()
        saveContinuation = nil
    }

    func events() -> [String] { recordedEvents }

    func overlayPresent() -> Bool { hasOverlay }
}

private actor DelayedSaveCloudClient: XboxCloudDataClient {
    private let scope: CloudCatalogScope

    init(scope: CloudCatalogScope) {
        self.scope = scope
    }

    func prepareAccess(
        session: StoredAuthSession, forceRegionIP: String,
        checkpoint: @escaping AuthCredentialCheckpoint,
        authProvider: (any XboxStreamingAuthProvider)?, startup: XboxStreamStartup?
    ) async throws -> PreparedCloudAccess {
        throw CancellationError()
    }

    func prepareHomeAccess(
        session: StoredAuthSession, forceRegionIP: String,
        checkpoint: @escaping AuthCredentialCheckpoint,
        authProvider: (any XboxStreamingAuthProvider)?, startup: XboxStreamStartup?
    ) async throws -> PreparedHomeAccess {
        throw CancellationError()
    }

    func loadCatalog(
        accessHandle _: String,
        market _: String,
        language _: String
    ) async throws -> RemoteCloudCatalogSnapshot {
        RemoteCloudCatalogSnapshot(
            games: [],
            scope: scope,
            fetchedAt: .now,
            failedHydrationChunks: 0,
            pendingHydrationProductIDs: []
        )
    }

    func loadMetadataPage(
        accessHandle _: String,
        market _: String,
        language _: String,
        productIDs _: [String]
    ) async throws -> [CloudCatalogMetadata] { [] }

    func releaseAccess(handle _: String) async {}
}
