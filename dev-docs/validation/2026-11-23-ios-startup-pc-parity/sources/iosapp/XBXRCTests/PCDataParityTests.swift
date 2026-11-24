import Foundation
import XCTest

@testable import XBXRC

final class PCDataParityTests: XCTestCase {
    func testStorageOverviewUsesCompleteDeviceCapacities() throws {
        let devices = [
            XboxHostStorageSummary(id: "a", name: "内部", freeBytes: 100, totalBytes: 1_000),
            XboxHostStorageSummary(id: "b", name: "扩展", freeBytes: 300, totalBytes: 2_000),
        ]
        let overview = try XCTUnwrap(PersonalLibraryPresentation.storageOverview(devices))
        XCTAssertEqual(overview.freeBytes, 400)
        XCTAssertEqual(overview.totalBytes, 3_000)
        XCTAssertEqual(overview.usedFraction ?? 0, 2_600.0 / 3_000, accuracy: 0.0001)
        XCTAssertNil(PersonalLibraryPresentation.storageOverview([]))
        XCTAssertNil(
            PersonalLibraryPresentation.storageOverview([
                devices[0],
                XboxHostStorageSummary(id: "c", name: "同步中", freeBytes: nil, totalBytes: nil),
            ]))
    }

    func testStorageOverviewClampsFreeSpaceAndHandlesCapacityOverflow() {
        let oversized = XboxHostStorageSummary(id: "a", name: "内部", freeBytes: 200, totalBytes: 100)
        XCTAssertEqual(PersonalLibraryPresentation.storageOverview([oversized])?.freeBytes, 100)
        XCTAssertNil(
            PersonalLibraryPresentation.storageOverview([
                oversized,
                XboxHostStorageSummary(id: "b", name: "扩展", freeBytes: 1, totalBytes: .max),
            ]))
    }

    @MainActor
    func testLateSelfHistoryPageCannotAppendAfterRefresh() async throws {
        let delayed = PCResponseGate<XboxHistoryPage>()
        let firstPages = PCHistorySequence()
        var client = ScriptedPCDataClient()
        client.history = { _, _, continuation in
            if continuation != nil { return await delayed.wait() }
            return await firstPages.next()
        }
        let store = XboxDataStore(client: client)
        await store.sync(session: PCPageFixtures.session, ownerGeneration: 1)
        await store.loadHistory()
        let more = Task { await store.loadHistory(nextPage: true) }
        try await waitFor { await delayed.wasRequested }
        await store.refreshLibrary()
        await delayed.resolve(XboxHistoryPage(items: [PCPageFixtures.games[2]], continuation: nil))
        await more.value
        XCTAssertEqual(store.games.map(\.titleID), ["2"])
        XCTAssertNil(store.libraryContinuation)
        XCTAssertFalse(store.isRefreshingLibrary)
    }

    @MainActor
    func testInstalledLanguageSwitchDiscardsLateEnglishResult() async throws {
        let delayed = PCResponseGate<XboxInstalledLibrarySnapshot>()
        var client = ScriptedPCDataClient()
        client.installed = { _, console, market, language in
            if language == "en-US" { return await delayed.wait() }
            return try await PCPageDataClient().loadInstalledLibrary(
                webTokenJSON: "fixture", consoleID: console, market: market, language: language)
        }
        let store = XboxDataStore(client: client)
        await store.sync(session: PCPageFixtures.session, ownerGeneration: 1)
        let english = Task {
            await store.loadInstalledLibrary(consoleID: "console", market: "US", language: "en-US")
        }
        try await waitFor { await delayed.wasRequested }
        await store.loadInstalledLibrary(consoleID: "console", market: "CN", language: "zh-CN")
        await delayed.resolve(
            XboxInstalledLibrarySnapshot(
                consoleID: "console", games: [], storage: [], powerState: "Off", notices: []))
        await english.value
        let state = try XCTUnwrap(store.installedLibraryStates["console"])
        XCTAssertEqual(state.scopeID, "CN:zh-cn")
        XCTAssertEqual(state.snapshot?.games?.count, 3)
        XCTAssertEqual(state.snapshot?.powerState, "On")
        XCTAssertFalse(state.isRefreshing)
    }

    @MainActor
    func testAccountChangeDiscardsLateTargetPlayerData() async throws {
        let delayed = PCResponseGate<XboxHistoryPage>()
        var client = ScriptedPCDataClient()
        client.history = { _, _, _ in await delayed.wait() }
        let store = XboxDataStore(client: client)
        await store.sync(session: PCPageFixtures.session, ownerGeneration: 1)
        let history = Task { await store.loadHistory(targetID: "2000") }
        try await waitFor { await delayed.wasRequested }
        await store.sync(session: PCPageFixtures.session, ownerGeneration: 2)
        await delayed.resolve(XboxHistoryPage(items: PCPageFixtures.games, continuation: nil))
        await history.value
        XCTAssertTrue(store.playerHistoryStates.isEmpty)
        XCTAssertTrue(store.nativeRequestIDs.isEmpty)
    }

    @MainActor
    func testRepeatedTargetContinuationPreservesCommittedPage() async {
        var client = ScriptedPCDataClient()
        client.history = { _, _, continuation in
            XboxHistoryPage(
                items: [PCPageFixtures.games[continuation == nil ? 0 : 1]], continuation: "repeat")
        }
        let store = XboxDataStore(client: client)
        await store.sync(session: PCPageFixtures.session, ownerGeneration: 1)
        await store.loadHistory(targetID: "2000")
        await store.loadHistory(targetID: "2000", nextPage: true)
        let state = store.historyState(targetID: "2000")
        XCTAssertEqual(state.items.map(\.titleID), ["1"])
        XCTAssertEqual(state.phase, .loaded)
        XCTAssertNotNil(state.errorMessage)
        XCTAssertFalse(state.isRefreshing)
    }

    @MainActor
    func testPartialTargetPlaytimeFailureRetainsPreviousValue() async {
        var client = ScriptedPCDataClient()
        client.playtimes = { _, titles, _ in
            XboxTitlePlaytimeSnapshot(
                items: titles.filter { $0 != "2" }.map { TitlePlaytime(titleID: $0, minutes: 150) },
                failedTitleIDs: ["2"])
        }
        let store = XboxDataStore(client: client)
        await store.sync(session: PCPageFixtures.session, ownerGeneration: 1)
        await store.loadHistory(targetID: "2000")
        let identity = XboxPersonalQueryKey.game("2", targetID: "2000")
        store.playerPlaytimeStates[identity] = XboxSingleDataState(value: 90, phase: .loaded)
        await store.loadHistoryPlaytimes(targetID: "2000", refresh: true)
        XCTAssertEqual(store.historyState(targetID: "2000").items.first?.playtimeMinutes, 150)
        XCTAssertEqual(store.playerPlaytimeStates[identity]?.value, 90)
        XCTAssertNotNil(store.playerPlaytimeStates[identity]?.errorMessage)
    }

    func testAchievementComparisonKeepsServiceConfigurationIdentity() {
        let own = PCPageFixtures.achievements(titleID: "1", targetID: nil)
        let friend = own.map { item in
            AchievementSummary(
                id: item.id, titleID: item.titleID, name: item.name, description: item.description,
                imageURL: nil, isSecret: item.isSecret, isUnlocked: true,
                gamerscore: item.gamerscore,
                progressPercentage: 100, unlockedAt: nil,
                scid: item.id == "secret" ? "other" : item.scid)
        }
        let rows = PersonalLibraryPresentation.achievementRows(own: own, friend: friend)
        XCTAssertEqual(rows.count, 3)
        XCTAssertEqual(Set(rows.map(\.id)).count, 3)
        XCTAssertNotNil(rows.first { $0.id == "core:first" }?.friend)
        XCTAssertNil(rows.first { $0.id == "core:secret" }?.friend)
        XCTAssertNil(rows.first { $0.id == "other:secret" }?.own)
    }

    @MainActor
    func testAlbumRenewalUsesOriginalSourcePage() async throws {
        let calls = PCRequestLog()
        var client = ScriptedPCDataClient()
        client.media = { _, kind, continuation in
            await calls.record(continuation ?? "first")
            let item = Self.media(
                kind: kind, id: continuation == nil ? "first" : "second", time: 100,
                url: URL(string: "https://example.invalid/fresh.png"))
            return XboxAlbumPage(items: [item], continuation: continuation == nil ? "page2" : nil)
        }
        let store = XboxDataStore(client: client)
        await store.sync(session: PCPageFixtures.session, ownerGeneration: 1)
        await store.loadAlbum(kind: .screenshot)
        await store.loadAlbum(kind: .screenshot, nextPage: true)
        let item = try XCTUnwrap(store.albumStates[.screenshot]?.items.last)
        let fresh = try await store.renewAlbumMedia(item)
        XCTAssertEqual(fresh.id, "second")
        XCTAssertEqual(fresh.pageContinuation, "page2")
        let requested = await calls.values
        XCTAssertEqual(requested, ["first", "page2", "page2"])
    }

    func testCombinedAlbumExpandsOnlyAfterBothSourcesCoverTimeRange() {
        let screenshots = XboxNativeDataState(
            items: [
                Self.media(kind: .screenshot, id: "same", time: 100),
                Self.media(kind: .screenshot, id: "old", time: 20),
            ], phase: .loaded, continuation: "screenshots-next")
        var clips = XboxNativeDataState(
            items: [Self.media(kind: .clip, id: "same", time: 80)],
            phase: .loaded, continuation: "clips-next")
        let first = PersonalLibraryPresentation.album([screenshots, clips])
        XCTAssertEqual(first.map(\.queryIdentity), ["screenshot:same", "clip:same"])
        clips.items.append(Self.media(kind: .clip, id: "earlier", time: 10))
        clips.continuation = nil
        let expanded = PersonalLibraryPresentation.album([screenshots, clips])
        XCTAssertEqual(
            expanded.map(\.queryIdentity), ["screenshot:same", "clip:same", "screenshot:old"])
    }

    @MainActor
    func testFavoritesRestoreOrderPinsMetadataAndAccountIsolation() throws {
        let suite = "XBXRC.PCFavorites.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let games = PCPageFixtures.cloudGames()
        defaults.set(
            [games[0].productID, games[0].productID, games[1].productID],
            forKey: "ios.cloud.favorites.1000")
        let store = CloudFavoritesStore(defaults: defaults)
        store.bind(accountID: "1000")
        XCTAssertEqual(store.order, games.prefix(2).map(\.productID))
        store.updateSnapshots(from: games)
        store.toggle(games[2])
        store.move(games[2].productID, by: -1)
        store.togglePin(games[1])
        store.setBrowserPreferences(category: "动作", sort: .source)
        let restored = CloudFavoritesStore(defaults: defaults)
        restored.bind(accountID: "1000")
        XCTAssertEqual(
            restored.order, [games[0].productID, games[2].productID, games[1].productID])
        XCTAssertEqual(
            restored.games(in: []).map(\.productID),
            [games[1].productID, games[0].productID, games[2].productID])
        XCTAssertTrue(
            restored.games(in: []).allSatisfy {
                $0.streamTitleID == nil && $0.hasEntitlement == nil
            })
        XCTAssertEqual(restored.category, "动作")
        XCTAssertEqual(restored.sort, .source)
        restored.bind(accountID: "another-account")
        XCTAssertTrue(restored.games(in: games).isEmpty)
        restored.bind(accountID: nil)
        XCTAssertTrue(restored.snapshots.isEmpty)
    }

    func testCatalogDiskRoundtripKeepsDiscoveryRanksAndUnavailableSources() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(
            "PCParity-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = CloudCatalogSnapshotRepository(rootDirectory: root)
        let scope = CloudCatalogScope(
            accountID: "1000", regionHost: "region.example", language: "en-US", market: "US")
        let snapshot = CloudCatalogSnapshot(
            scope: scope, games: PCPageFixtures.cloudGames(),
            baseUpdatedAt: .now, overlayUpdatedAt: .now, unavailableSources: ["popular", "leaving"])
        try await repository.save(snapshot)
        let loadedValue = try await repository.load(scope: scope)
        let loaded = try XCTUnwrap(loadedValue)
        XCTAssertEqual(loaded.games, snapshot.games)
        XCTAssertEqual(loaded.unavailableSources, ["popular", "leaving"])
        var legacy = try XCTUnwrap(
            JSONSerialization.jsonObject(with: JSONEncoder().encode(snapshot)) as? [String: Any])
        legacy.removeValue(forKey: "unavailableSources")
        let decoded = try JSONDecoder().decode(
            CloudCatalogSnapshot.self, from: JSONSerialization.data(withJSONObject: legacy))
        XCTAssertNil(decoded.unavailableSources)
    }

    @MainActor
    func testProductLanguageSwitchDiscardsLateOldLanguageResponse() async throws {
        let suite = "XBXRC.PCProductScope.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        settings.mobile.catalogLanguage = "en-US"
        let delayed = PCResponseGate<CloudProductDetails>()
        var client = ScriptedPCCloudClient()
        client.detail = { product, market, language in
            if language == "en-US" { return await delayed.wait() }
            return try await PCPageCloudClient().loadProductDetail(
                productID: product, market: market, language: language)
        }
        let store = CloudLibraryStore(client: client, settingsProvider: settings)
        let productID = PCPageFixtures.cloudGames()[0].productID
        let english = Task { await store.loadProductDetail(productID: productID) }
        try await waitFor { await delayed.wasRequested }
        settings.mobile.catalogLanguage = "zh-TW"
        await store.loadProductDetail(productID: productID)
        let fresh = try XCTUnwrap(store.productDetailState(productID).value)
        await delayed.resolve(
            try await PCPageCloudClient().loadProductDetail(
                productID: productID, market: "US", language: "en-US"))
        await english.value
        XCTAssertEqual(store.productDetailState(productID).value, fresh)
        XCTAssertFalse(store.productDetailState(productID).isRefreshing)
        settings.mobile.catalogLanguage = "en-US"
        XCTAssertEqual(store.productDetailState(productID).phase, .idle)
    }

    @MainActor
    private func waitFor(_ condition: @escaping @MainActor () async -> Bool) async throws {
        let deadline = ContinuousClock().now.advanced(by: .seconds(2))
        while !(await condition()) {
            guard ContinuousClock().now < deadline else { throw PCParityError.timeout }
            try await Task.sleep(for: .milliseconds(5))
        }
    }

    private static func media(kind: XboxAlbumKind, id: String, time: TimeInterval, url: URL? = nil)
        -> XboxAlbumMedia
    {
        XboxAlbumMedia(
            id: id, kind: kind, titleID: "1", titleName: "Halo Infinite",
            capturedAt: Date(timeIntervalSince1970: time), durationSeconds: nil, thumbnailURL: nil,
            url: url, expiresAt: nil)
    }
}

private enum PCParityError: Error { case timeout }

private actor PCResponseGate<Value: Sendable> {
    private var value: Value?
    private var waiters: [CheckedContinuation<Value, Never>] = []
    private(set) var wasRequested = false
    func wait() async -> Value {
        wasRequested = true
        if let value { return value }
        return await withCheckedContinuation { waiters.append($0) }
    }
    func resolve(_ value: Value) {
        self.value = value
        for waiter in waiters { waiter.resume(returning: value) }
        waiters.removeAll()
    }
}

private actor PCHistorySequence {
    private var count = 0
    func next() -> XboxHistoryPage {
        count += 1
        return XboxHistoryPage(
            items: [PCPageFixtures.games[count == 1 ? 0 : 1]],
            continuation: count == 1 ? "page2" : nil)
    }
}

private actor PCRequestLog {
    private(set) var values: [String] = []
    func record(_ value: String) { values.append(value) }
}

private struct ScriptedPCDataClient: XboxDataClient {
    var section: (@Sendable (XboxInstalledSource, String) async throws -> InstalledSectionResult)?
    func loadInstalledSection(
        webTokenJSON: String, consoleID: String, market: String, language: String,
        source: XboxInstalledSource, productIDs: [String]
    ) async throws -> InstalledSectionResult {
        if let section { return try await section(source, consoleID) }
        return InstalledSectionResult(
            snapshot: try await loadInstalledLibrary(
                webTokenJSON: webTokenJSON,
                consoleID: consoleID, market: market, language: language))
    }
    var history: (@Sendable (String, String?, String?) async throws -> XboxHistoryPage)?
    var installed:
        (@Sendable (String, String, String, String) async throws -> XboxInstalledLibrarySnapshot)?
    var playtimes:
        (@Sendable (String, [String], String?) async throws -> XboxTitlePlaytimeSnapshot)?
    var media: (@Sendable (String, XboxAlbumKind, String?) async throws -> XboxAlbumPage)?
    func loadHistoryPage(webTokenJSON: String, targetID: String?, continuation: String?)
        async throws -> XboxHistoryPage
    {
        if let history { return try await history(webTokenJSON, targetID, continuation) }
        return try await PCPageDataClient().loadHistoryPage(
            webTokenJSON: webTokenJSON, targetID: targetID, continuation: continuation)
    }
    func loadInstalledLibrary(
        webTokenJSON: String, consoleID: String, market: String, language: String
    ) async throws -> XboxInstalledLibrarySnapshot {
        if let installed { return try await installed(webTokenJSON, consoleID, market, language) }
        return try await PCPageDataClient().loadInstalledLibrary(
            webTokenJSON: webTokenJSON, consoleID: consoleID, market: market, language: language)
    }
    func loadPlayerPlaytimes(webTokenJSON: String, titleIDs: [String], targetID: String?)
        async throws -> XboxTitlePlaytimeSnapshot
    {
        if let playtimes { return try await playtimes(webTokenJSON, titleIDs, targetID) }
        return try await PCPageDataClient().loadPlayerPlaytimes(
            webTokenJSON: webTokenJSON, titleIDs: titleIDs, targetID: targetID)
    }
    func loadMedia(webTokenJSON: String, kind: XboxAlbumKind, continuation: String?) async throws
        -> XboxAlbumPage
    {
        if let media { return try await media(webTokenJSON, kind, continuation) }
        return try await PCPageDataClient().loadMedia(
            webTokenJSON: webTokenJSON, kind: kind, continuation: continuation)
    }
    func loadPlayerProfile(webTokenJSON: String, targetID: String) async throws -> XboxFriendSummary
    {
        try await PCPageDataClient().loadPlayerProfile(
            webTokenJSON: webTokenJSON, targetID: targetID)
    }
    func loadLibraryGame(webTokenJSON: String, titleID: String, targetID: String?) async throws
        -> GameSummary?
    {
        try await PCPageDataClient().loadLibraryGame(
            webTokenJSON: webTokenJSON, titleID: titleID, targetID: targetID)
    }
    func loadAchievementPage(
        webTokenJSON: String, titleID: String, targetID: String?, locale: String,
        continuation: String?
    ) async throws -> XboxAchievementsPage {
        try await PCPageDataClient().loadAchievementPage(
            webTokenJSON: webTokenJSON, titleID: titleID, targetID: targetID, locale: locale,
            continuation: continuation)
    }
    func loadFriends(webTokenJSON: String) async throws -> [XboxFriendSummary] {
        try await PCPageDataClient().loadFriends(webTokenJSON: webTokenJSON)
    }
    func loadHosts(webTokenJSON: String) async throws -> [XboxHostSummary] {
        try await PCPageDataClient().loadHosts(webTokenJSON: webTokenJSON)
    }
    func powerOn(webTokenJSON: String, consoleID: String) async throws -> HostPowerCommandResult {
        try await PCPageDataClient().powerOn(webTokenJSON: webTokenJSON, consoleID: consoleID)
    }
    func powerOff(webTokenJSON: String, consoleID: String) async throws -> HostPowerCommandResult {
        try await PCPageDataClient().powerOff(webTokenJSON: webTokenJSON, consoleID: consoleID)
    }
    func loadGameLibrary(webTokenJSON: String) async throws -> [GameSummary] {
        try await PCPageDataClient().loadGameLibrary(webTokenJSON: webTokenJSON)
    }
    func loadPlaytimes(webTokenJSON: String, titleIDs: [String]) async throws -> [TitlePlaytime] {
        try await PCPageDataClient().loadPlaytimes(webTokenJSON: webTokenJSON, titleIDs: titleIDs)
    }
    func loadAchievements(webTokenJSON: String, titleID: String, locale: String) async throws
        -> [AchievementSummary]
    {
        try await PCPageDataClient().loadAchievements(
            webTokenJSON: webTokenJSON, titleID: titleID, locale: locale)
    }
}

private struct ScriptedPCCloudClient: XboxCloudDataClient {
    var catalog: (@Sendable (String, String) async throws -> RemoteCloudCatalogSnapshot)?
    var source: (@Sendable (String) async throws -> [String])?
    func loadCatalogSource(accessHandle: String, source: String, market: String, language: String)
        async throws -> [String]
    {
        if let handler = self.source { return try await handler(source) }
        return PCPageFixtures.cloudGames().map(\.productID)
    }
    var detail: (@Sendable (String, String, String) async throws -> CloudProductDetails)?
    func loadProductDetail(productID: String, market: String, language: String) async throws
        -> CloudProductDetails
    {
        if let detail { return try await detail(productID, market, language) }
        return try await PCPageCloudClient().loadProductDetail(
            productID: productID, market: market, language: language)
    }
    func prepareAccess(
        session: StoredAuthSession, forceRegionIP: String,
        checkpoint: @escaping AuthCredentialCheckpoint,
        authProvider: (any XboxStreamingAuthProvider)?, startup: XboxStreamStartup?
    ) async throws
        -> PreparedCloudAccess
    {
        try await PCPageCloudClient().prepareAccess(
            session: session, forceRegionIP: forceRegionIP, checkpoint: checkpoint,
            authProvider: authProvider, startup: startup)
    }
    func prepareHomeAccess(
        session: StoredAuthSession, forceRegionIP: String,
        checkpoint: @escaping AuthCredentialCheckpoint,
        authProvider: (any XboxStreamingAuthProvider)?, startup: XboxStreamStartup?
    )
        async throws -> PreparedHomeAccess
    {
        try await PCPageCloudClient().prepareHomeAccess(
            session: session, forceRegionIP: forceRegionIP, checkpoint: checkpoint,
            authProvider: authProvider, startup: startup)
    }
    func loadCatalog(accessHandle: String, market: String, language: String) async throws
        -> RemoteCloudCatalogSnapshot
    {
        if let catalog { return try await catalog(market, language) }
        return try await PCPageCloudClient().loadCatalog(
            accessHandle: accessHandle, market: market, language: language)
    }
    func loadMetadataPage(
        accessHandle: String, market: String, language: String, productIDs: [String]
    ) async throws -> [CloudCatalogMetadata] {
        try await PCPageCloudClient().loadMetadataPage(
            accessHandle: accessHandle, market: market, language: language, productIDs: productIDs)
    }
    func releaseAccess(handle: String) async {}
}

extension PCDataParityTests {
    @MainActor
    func testConcurrentSourceRetriesKeepBothRanksAndSavedTimes() async throws {
        let calls = PCRequestLog()
        let gate = PCResponseGate<Bool>()
        let repository = PCSourceSnapshotRepository(gate: gate)
        var client = ScriptedPCCloudClient()
        client.catalog = { market, language in
            var games = PCPageFixtures.cloudGames()
            for i in games.indices {
                games[i].popularRank = nil
                games[i].newRank = nil
                games[i].isNew = false
            }
            return RemoteCloudCatalogSnapshot(
                games: games,
                scope: CloudCatalogScope(
                    accountID: "1000", regionHost: "uswest.gssv-play-prod.xboxlive.com",
                    language: language, market: market),
                fetchedAt: .now, failedHydrationChunks: 0, pendingHydrationProductIDs: [],
                unavailableSources: ["popular", "newest"])
        }
        client.source = { source in
            await calls.record(source)
            return [source == "popular" ? "HALO00000001" : "SEA000000001"]
        }
        let store = CloudLibraryStore(client: client, repository: repository)
        let access: @MainActor () async throws -> PreparedCloudAccess = {
            try await PCPageCloudClient().prepareAccess(
                session: StoredAuthSession(bridgeSession: PCPageFixtures.bridgeSession),
                forceRegionIP: "",
                checkpoint: { _ in }, authProvider: nil, startup: nil)
        }
        await store.refresh(reason: .initialActivation, prepareAccess: access)
        await repository.delayNextSave()
        let popular = Task { await store.retrySource("popular", prepareAccess: access) }
        try await waitFor { await gate.wasRequested }
        let newest = Task { await store.retrySource("newest", prepareAccess: access) }
        try await waitFor { await calls.values.contains("newest") }
        await gate.resolve(true)
        await popular.value
        await newest.value
        XCTAssertEqual(store.games.first { $0.productID == "HALO00000001" }?.popularRank, 0)
        XCTAssertEqual(store.games.first { $0.productID == "SEA000000001" }?.newRank, 0)
        XCTAssertFalse(store.unavailableSources.contains("popular"))
        XCTAssertFalse(store.unavailableSources.contains("newest"))
        let saved = await repository.snapshot
        XCTAssertNotNil(saved?.sourceUpdatedAt?["popular"])
        XCTAssertNotNil(saved?.sourceUpdatedAt?["newest"])
        XCTAssertEqual(saved?.games, store.games)
    }

    @MainActor
    func testSectionRetryLoadsOnlyFailedInstalledSource() async throws {
        let calls = PCRequestLog()
        var client = ScriptedPCDataClient()
        client.section = { source, console in
            await calls.record(String(describing: source))
            if source == .storage, await calls.values.filter({ $0 == "storage" }).count == 1 {
                throw SectionRequestFailure(kind: .timeout)
            }
            return InstalledSectionResult(
                snapshot: try await PCPageDataClient().loadInstalledLibrary(
                    webTokenJSON: "fixture", consoleID: console, market: "US", language: "en-US"))
        }
        let store = XboxDataStore(client: client)
        await store.sync(session: PCPageFixtures.session, ownerGeneration: 1)
        await store.loadInstalledLibrary(consoleID: "console", market: "US", language: "en-US")
        let failed = try XCTUnwrap(store.installedLibraryStates["console"])
        XCTAssertEqual(failed.sections[.storage]?.failure?.kind, .timeout)
        XCTAssertEqual(failed.snapshot?.games?.count, 3)
        XCTAssertFalse(failed.isRefreshing)
        let previous = await calls.values
        await store.loadInstalledSection(
            consoleID: "console", market: "US", language: "en-US", source: .storage)
        let after = await calls.values
        XCTAssertEqual(Array(after.dropFirst(previous.count)), ["storage"])
        XCTAssertNil(store.installedLibraryStates["console"]?.sections[.storage]?.failure)
        XCTAssertEqual(store.installedLibraryStates["console"]?.snapshot?.games?.count, 3)
    }

    @MainActor
    func testEmptySuccessRefreshFailurePreservesSnapshotAndTime() async throws {
        let calls = PCRequestLog()
        var client = ScriptedPCDataClient()
        client.history = { _, _, _ in
            await calls.record("history")
            if await calls.values.count > 1 { throw SectionRequestFailure(kind: .network) }
            return XboxHistoryPage(items: [], continuation: nil)
        }
        let store = XboxDataStore(client: client)
        await store.sync(session: PCPageFixtures.session, ownerGeneration: 1)
        await store.loadHistory(targetID: "2000")
        let date = try XCTUnwrap(store.historyState(targetID: "2000").updatedAt)
        await store.loadHistory(targetID: "2000", refresh: true)
        let state = store.historyState(targetID: "2000")
        XCTAssertTrue(state.hasSnapshot)
        XCTAssertTrue(state.items.isEmpty)
        XCTAssertEqual(state.updatedAt, date)
        XCTAssertEqual(state.requestAttempt, .refresh)
        XCTAssertEqual(state.failure?.kind, .network)
        XCTAssertFalse(state.isRefreshing)
    }

    @MainActor
    func testPageFailureRetriesOriginalContinuation() async throws {
        let calls = PCRequestLog()
        var client = ScriptedPCDataClient()
        client.history = { _, _, continuation in
            await calls.record(continuation ?? "first")
            if continuation != nil, await calls.values.filter({ $0 == "page2" }).count == 1 {
                throw SectionRequestFailure(kind: .serviceUnavailable)
            }
            return XboxHistoryPage(
                items: [PCPageFixtures.games[continuation == nil ? 0 : 1]],
                continuation: continuation == nil ? "page2" : nil)
        }
        let store = XboxDataStore(client: client)
        await store.sync(session: PCPageFixtures.session, ownerGeneration: 1)
        await store.loadHistory(targetID: "2000")
        await store.loadHistory(targetID: "2000", nextPage: true)
        let failed = store.historyState(targetID: "2000")
        XCTAssertEqual(failed.continuation, "page2")
        XCTAssertEqual(failed.items.map(\.titleID), ["1"])
        XCTAssertEqual(failed.requestAttempt, .nextPage)
        await store.loadHistory(targetID: "2000", nextPage: true)
        let requested = await calls.values
        XCTAssertEqual(requested, ["first", "page2", "page2"])
        XCTAssertEqual(store.historyState(targetID: "2000").items.map(\.titleID), ["1", "2"])
        XCTAssertNil(store.historyState(targetID: "2000").failure)
    }

    @MainActor
    func testCancelledPlaytimeEndsBusyWithoutFailure() async {
        var client = ScriptedPCDataClient()
        client.playtimes = { _, _, _ in throw URLError(.cancelled) }
        let store = XboxDataStore(client: client)
        await store.sync(session: PCPageFixtures.session, ownerGeneration: 1)
        await store.loadHistory(targetID: "2000")
        await store.loadHistoryPlaytimes(targetID: "2000")
        XCTAssertFalse(store.historyPlaytimeStates["2000"]?.isRefreshing ?? true)
        XCTAssertNil(store.historyPlaytimeStates["2000"]?.failure)
        XCTAssertEqual(store.historyPlaytimeStates["2000"]?.phase, .idle)
        XCTAssertTrue(store.nativeRequestIDs.isEmpty)
    }

    @MainActor
    func testAlbumPartialFailurePublishesSuccessfulSourceImmediately() async {
        let calls = PCRequestLog()
        var client = ScriptedPCDataClient()
        client.media = { _, kind, _ in
            await calls.record(kind.rawValue)
            if kind == .screenshot, await calls.values.filter({ $0 == "screenshot" }).count == 1 {
                throw SectionRequestFailure(kind: .timeout)
            }
            return XboxAlbumPage(
                items: [Self.media(kind: kind, id: kind.rawValue, time: 0)], continuation: nil)
        }
        let store = XboxDataStore(client: client)
        await store.sync(session: PCPageFixtures.session, ownerGeneration: 1)
        async let screenshot: Void = store.loadAlbum(kind: .screenshot)
        async let clip: Void = store.loadAlbum(kind: .clip)
        _ = await (screenshot, clip)
        XCTAssertEqual(
            PersonalLibraryPresentation.album(
                XboxAlbumKind.allCases.map { store.albumStates[$0] ?? XboxNativeDataState() }
            ).count, 1)
        await store.loadAlbum(kind: .screenshot, refresh: true)
        let values = await calls.values
        XCTAssertEqual(values.filter { $0 == "clip" }.count, 1)
        XCTAssertEqual(store.albumStates[.screenshot]?.items.count, 1)
        XCTAssertNil(store.albumStates[.screenshot]?.failure)
    }

    func testTechnicalErrorsUseFiniteLocalizedMessages() {
        let privateError = XboxBridgeError.Data(
            "https://private.invalid?token=secret statusCode=500")
        XCTAssertEqual(SectionRequestFailure(error: privateError).kind, .unknown)
        XCTAssertFalse(SectionRequestFailure(error: privateError).message.contains("secret"))
        for (kind, expected) in [
            (XboxQueryFailureKind.network, SectionRequestFailure.Kind.network),
            (.timeout, .timeout), (.authentication, .authentication),
            (.accessRestricted, .accessRestricted),
            (.rateLimited, .rateLimited), (.serviceUnavailable, .serviceUnavailable),
            (.invalidResponse, .invalidResponse),
        ] {
            let failure = XboxQueryFailure(
                kind: kind, statusCode: nil, retriable: true, retryAfterSeconds: 30)
            XCTAssertEqual(SectionRequestFailure(failure).kind, expected)
            XCTAssertNotNil(SectionRequestFailure(failure).retryAt)
        }
        XCTAssertEqual(appLocalizedString("重试更新", locale: Locale(identifier: "en")), "Retry update")
        XCTAssertEqual(
            appLocalizedString("显示上次内容", locale: Locale(identifier: "en")),
            "Showing previous content")
        XCTAssertEqual(
            appLocalizedFormat("显示上次内容 · %@", locale: Locale(identifier: "en"), "10:30"),
            "Showing previous content · 10:30")
    }
}

private actor PCSourceSnapshotRepository: CloudCatalogSnapshotStoring {
    let gate: PCResponseGate<Bool>
    private var delaySave = false
    private(set) var snapshot: CloudCatalogSnapshot?
    init(gate: PCResponseGate<Bool>) { self.gate = gate }
    func delayNextSave() { delaySave = true }
    func load(scope: CloudCatalogScope) async throws -> CloudCatalogSnapshot? {
        snapshot?.scope == scope ? snapshot : nil
    }
    func save(_ snapshot: CloudCatalogSnapshot) async throws {
        if delaySave {
            delaySave = false
            _ = await gate.wait()
        }
        self.snapshot = snapshot
    }
    func clearOverlay(accountID: String) async throws {
        if snapshot?.scope.accountID == accountID { snapshot = nil }
    }
}

extension PCDataParityTests {
    @MainActor
    func testCloudRetryChangesOnlyFailedSourceAndPersistsItsSuccessTime() async throws {
        let calls = PCRequestLog()
        var client = ScriptedPCCloudClient()
        client.catalog = { market, language in
            var games = PCPageFixtures.cloudGames()
            for i in games.indices { games[i].popularRank = nil }
            return RemoteCloudCatalogSnapshot(
                games: games,
                scope: CloudCatalogScope(
                    accountID: "1000", regionHost: "uswest.gssv-play-prod.xboxlive.com",
                    language: language, market: market),
                fetchedAt: .now, failedHydrationChunks: 0, pendingHydrationProductIDs: [],
                unavailableSources: ["popular"],
                sourceFailures: ["popular": SectionRequestFailure(kind: .timeout)])
        }
        client.source = { source in
            await calls.record(source)
            return ["SEA000000001", "HALO00000001"]
        }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
            "section-catalog-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: directory) }
        let repository = CloudCatalogSnapshotRepository(rootDirectory: directory)
        let store = CloudLibraryStore(client: client, repository: repository)
        await store.refresh(reason: .initialActivation) {
            try await PCPageCloudClient().prepareAccess(
                session: StoredAuthSession(bridgeSession: PCPageFixtures.bridgeSession),
                forceRegionIP: "",
                checkpoint: { _ in }, authProvider: nil, startup: nil)
        }
        let recentRanks = store.games.map(\.recentRank)
        let recentTime = store.sourceStates["recent"]?.updatedAt
        XCTAssertEqual(store.sourceStates["popular"]?.failure?.kind, .timeout)
        await store.retrySource("popular") {
            try await PCPageCloudClient().prepareAccess(
                session: StoredAuthSession(bridgeSession: PCPageFixtures.bridgeSession),
                forceRegionIP: "",
                checkpoint: { _ in }, authProvider: nil, startup: nil)
        }
        let requested = await calls.values
        XCTAssertEqual(requested, ["popular"])
        XCTAssertEqual(store.games.map(\.recentRank), recentRanks)
        XCTAssertEqual(store.sourceStates["recent"]?.updatedAt, recentTime)
        XCTAssertNil(store.sourceStates["popular"]?.failure)
        XCTAssertEqual(store.games.last?.popularRank, 0)
        let scope = CloudCatalogScope(
            accountID: "1000", regionHost: "uswest.gssv-play-prod.xboxlive.com", language: "en-US",
            market: "US")
        let saved = try await repository.load(scope: scope)
        // 测试设备可使用中文目录语言，使用当前合同构造 scope。
        let chinese = try await repository.load(
            scope: CloudCatalogScope(
                accountID: scope.accountID,
                regionHost: scope.regionHost, language: "zh-TW", market: scope.market))
        let actual = saved ?? chinese
        XCTAssertNotNil(actual?.sourceUpdatedAt?["popular"])
        XCTAssertFalse(actual?.unavailableSources?.contains("popular") ?? true)
    }

    @MainActor
    func testConcurrent401RequestsShareOneRenewalAndResumeOriginalSources() async throws {
        let client = SectionRenewalAuthClient()
        let settings = AppSettingsStore()
        let auth = AuthStore(
            settings: settings, client: client, cloudClient: PCPageCloudClient(),
            keychain: PCPageSessionStore())
        await auth.restore()
        let token = try XCTUnwrap(auth.session?.webTokenJSON)
        var dataClient = ScriptedPCDataClient()
        dataClient.history = { raw, _, _ in
            if raw == token { throw SectionRequestFailure(kind: .authentication) }
            return XboxHistoryPage(items: [PCPageFixtures.games[0]], continuation: nil)
        }
        dataClient.media = { raw, kind, _ in
            if raw == token { throw SectionRequestFailure(kind: .authentication) }
            return XboxAlbumPage(
                items: [Self.media(kind: kind, id: "renewed", time: 0)], continuation: nil)
        }
        let data = XboxDataStore(client: dataClient)
        data.authentication = auth
        await data.sync(session: auth.session, ownerGeneration: auth.ownerGeneration)
        async let history: Void = data.loadHistory(targetID: "2000")
        async let media: Void = data.loadAlbum(kind: .clip)
        _ = await (history, media)
        let renewals = await client.count
        XCTAssertEqual(renewals, 2)  // 启动恢复一次，两来源401共用一次。
        XCTAssertEqual(data.historyState(targetID: "2000").items.count, 1)
        XCTAssertEqual(data.albumStates[.clip]?.items.count, 1)
        XCTAssertFalse(auth.needsDataSignIn)
    }
}

private actor SectionRenewalAuthClient: XboxAuthClient {
    private(set) var count = 0
    func beginLogin() async throws -> LoginStartResult { throw XboxNativeQueryError.unavailable }
    func finishLogin(callbackURL: URL, pendingJSON: String, seedJSON: String, forceRegionIP: String)
        async throws -> AuthSession
    {
        PCPageFixtures.bridgeSession
    }
    func renewLogin(
        session: StoredAuthSession, forceRegionIP: String, scope: XboxAuthScope,
        forceCoreRefresh: Bool, rejectedToken: String?,
        checkpoint: @escaping AuthCredentialCheckpoint
    ) async throws
        -> AuthSession
    {
        count += 1
        let number = count
        try await Task.sleep(for: .milliseconds(30))
        return AuthSession(
            refreshToken: "fixture-refresh", seedJson: "{}", webTokenJson: "renewed-\(number)",
            appLevel: 0)
    }
    func loadProfile(webTokenJSON: String) async throws -> XboxProfile { PCPageFixtures.profile }
}
