import XCTest
@testable import XBXRC

final class XBXRCTests: XCTestCase {
    func testGameSummaryKeepsUnavailablePlaytimeDistinct() {
        let game = GameSummary(
            id: "halo-infinite",
            titleID: "1292135258",
            name: "Halo Infinite",
            artworkURL: nil,
            playtimeMinutes: nil,
            achievementProgress: nil
        )

        XCTAssertNil(game.playtimeMinutes)
    }

    func testXboxPresentationFormatsPlaytime() {
        XCTAssertEqual(XboxPresentation.playtime(nil), "时长未知")
        XCTAssertEqual(XboxPresentation.playtime(45), "45 分钟")
        XCTAssertEqual(XboxPresentation.playtime(90), "1.5 小时")
        XCTAssertEqual(XboxPresentation.playtime(725), "12 小时")
    }

    func testXboxImageURLNormalizesSecureSchemes() {
        XCTAssertEqual(
            XboxImageURL.resolve("http://images.example.com/game.jpg")?.absoluteString,
            "https://images.example.com/game.jpg"
        )
        XCTAssertEqual(
            XboxImageURL.resolve("//images.example.com/game.jpg")?.absoluteString,
            "https://images.example.com/game.jpg"
        )
        XCTAssertEqual(
            XboxImageURL.resolve("https://images.example.com/game.jpg")?.absoluteString,
            "https://images.example.com/game.jpg"
        )
    }

    func testLibraryPresentationSortsRecentAndKeepsNilDatesInSourceOrder() {
        let older = Date(timeIntervalSince1970: 1_000)
        let newer = Date(timeIntervalSince1970: 2_000)
        let games = [
            makeGame(id: "nil-zulu", name: "Zulu", lastPlayedAt: nil),
            makeGame(id: "same-zulu", name: "Zulu", lastPlayedAt: older),
            makeGame(id: "newer", name: "Newest", lastPlayedAt: newer),
            makeGame(id: "same-alpha", name: "Alpha", lastPlayedAt: older),
            makeGame(id: "nil-alpha", name: "Alpha", lastPlayedAt: nil),
        ]

        let recent = LibraryPresentation.collections(from: games)
            .first { $0.kind == .recent }

        XCTAssertEqual(
            recent?.games.map(\.id),
            ["newer", "same-alpha", "same-zulu"]
        )
    }

    func testLibraryPresentationMatchesDesktopXcloudDimensions() {
        let games = [
            makeCloudGame(id: "recent", isRecentlyPlayed: true),
            makeCloudGame(id: "new", isNew: true),
            makeCloudGame(
                id: "activity-only",
                lastPlayedAt: Date(timeIntervalSince1970: 2_000)
            ),
            makeCloudGame(id: "all"),
        ]

        let collections = LibraryPresentation.collections(fromCloudGames: games)

        XCTAssertEqual(collections.map(\.kind), [.recent, .newlyAdded, .all])
        XCTAssertEqual(collections.map(\.title), ["最近游玩", "新入库", "全部云游戏"])
        XCTAssertEqual(collections.first { $0.kind == .recent }?.games.map(\.id), ["recent"])
    }

    func testLibraryPresentationTruncatesHomeAndHeroWithoutTruncatingCollection() {
        let games = (0..<10).map { index in
            makeGame(
                id: "game-\(index)",
                name: "Game \(index)",
                lastPlayedAt: Date(timeIntervalSince1970: TimeInterval(index))
            )
        }

        let recent = LibraryPresentation.collections(from: games)
            .first { $0.kind == .recent }

        XCTAssertEqual(recent?.games.count, 10)
        XCTAssertEqual(recent?.homeGames.count, 8)
        XCTAssertEqual(recent?.homeGames.map(\.id), Array(recent?.games.prefix(8).map(\.id) ?? []))
        XCTAssertEqual(LibraryPresentation.heroGames(from: games).map(\.id), [
            "game-9", "game-8", "game-7", "game-6", "game-5",
        ])
    }

    func testLibraryPresentationKeepsAllGamesAndHidesEmptyOptionalCollections() {
        let duplicateFirst = makeGame(id: "duplicate-first", name: "Same")
        let duplicateSecond = makeGame(id: "duplicate-second", name: "Same")
        let games = [
            makeGame(id: "zulu", name: "Zulu"),
            duplicateFirst,
            makeGame(id: "alpha", name: "Alpha"),
            duplicateSecond,
        ]

        let collections = LibraryPresentation.collections(from: games)
        let all = collections.first { $0.kind == .all }

        XCTAssertEqual(collections.map(\.kind), [.all])
        XCTAssertEqual(
            all?.games.map(\.id),
            ["alpha", "duplicate-first", "duplicate-second", "zulu"]
        )
        XCTAssertEqual(all?.games.count, games.count)
        XCTAssertEqual(LibraryPresentation.collections(from: []), [])
    }

    func testLibraryPresentationMetadataUsesCollectionDimension() {
        let game = makeCloudGame(
            id: "metadata",
            isNew: true
        )

        XCTAssertEqual(
            LibraryPresentation.metadata(for: game, kind: .newlyAdded),
            "Game Pass 新入库"
        )
        XCTAssertEqual(
            LibraryPresentation.metadata(
                for: makeGame(id: "fallback", name: "Fallback"),
                kind: .all
            ),
            "Xbox 游戏"
        )
    }

    func testStoredAuthSessionRoundTrip() throws {
        let session = StoredAuthSession(
            refreshToken: "refresh",
            seedJSON: "{\"seed\":true}",
            webTokenJSON: "{\"token\":true}",
            appLevel: 2
        )

        let encoded = try JSONEncoder().encode(session)
        XCTAssertEqual(try JSONDecoder().decode(StoredAuthSession.self, from: encoded), session)
    }

    @MainActor
    func testAppSettingsStorePersistsCloudRegionPreset() {
        let suiteName = "XBXRC.SettingsTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let store = AppSettingsStore(defaults: defaults)
        XCTAssertEqual(store.cloudRegionPreset, .default)
        XCTAssertTrue(store.setCloudRegionPreset(.japan))
        XCTAssertEqual(AppSettingsStore(defaults: defaults).cloudRegionPreset, .japan)
        XCTAssertEqual(store.cloudRegionPreset.forceRegionIP, "210.131.113.123")

        store.usesEphemeralLoginSession = true
        XCTAssertTrue(AppSettingsStore(defaults: defaults).usesEphemeralLoginSession)
    }

    func testCloudLibraryImageCandidatesPreferSuccessfulImageAndKeepFallbackOrder() {
        let game = makeCloudGame(
            id: "halo",
            heroURL: URL(string: "https://example.invalid/hero.jpg"),
            posterURL: URL(string: "https://example.invalid/poster.jpg"),
            tileURL: URL(string: "https://example.invalid/tile.jpg"),
            artworkURL: URL(string: "https://example.invalid/artwork.jpg")
        )
        let preferred = URL(string: "https://example.invalid/success.jpg")!

        XCTAssertEqual(
            game.imageCandidates(preferredURL: preferred).map(\.absoluteString),
            [
                preferred.absoluteString,
                "https://example.invalid/hero.jpg",
                "https://example.invalid/poster.jpg",
                "https://example.invalid/tile.jpg",
                "https://example.invalid/artwork.jpg",
            ]
        )
    }

    func testCloudCatalogSnapshotFreshStaleAndExpiredWindows() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let scope = CloudCatalogScope(
            accountID: "xid",
            regionHost: "region.example.com",
            language: "zh-CN",
            market: "CN"
        )

        let fresh = CloudCatalogSnapshot(
            scope: scope,
            games: [],
            baseUpdatedAt: now.addingTimeInterval(-60),
            overlayUpdatedAt: now.addingTimeInterval(-60)
        )
        let stale = CloudCatalogSnapshot(
            scope: scope,
            games: [],
            baseUpdatedAt: now.addingTimeInterval(-3_600),
            overlayUpdatedAt: now.addingTimeInterval(-601)
        )
        let expired = CloudCatalogSnapshot(
            scope: scope,
            games: [],
            baseUpdatedAt: now.addingTimeInterval(-3_600),
            overlayUpdatedAt: now.addingTimeInterval(-(24 * 60 * 60 + 1))
        )

        XCTAssertEqual(fresh.cacheState(at: now), .fresh)
        XCTAssertEqual(stale.cacheState(at: now), .stale)
        XCTAssertEqual(expired.cacheState(at: now), .expired)
    }

    func testCloudLibraryDiagnosticsRedactsSensitiveErrorDetails() {
        let error = NSError(
            domain: "CloudTest",
            code: 42,
            userInfo: [
                NSLocalizedDescriptionKey:
                    "request https://example.invalid/path?token=secret Bearer abc123 cloud-0123456789abcdef"
            ]
        )

        let summary = CloudLibraryDiagnostics.safeError(error)

        XCTAssertTrue(summary.contains("CloudTest#42"))
        XCTAssertFalse(summary.contains("https://"))
        XCTAssertFalse(summary.contains("abc123"))
        XCTAssertFalse(summary.contains("cloud-0123456789abcdef"))
    }

    func testCloudLibraryDiagnosticsProjectsStreamingOfferingFailure() {
        let error = NSError(
            domain: "XboxBridgeError",
            code: 1,
            userInfo: [
                NSLocalizedDescriptionKey:
                    "xCloud token is unavailable; forceRegionApplied=false; "
                    + "offering=xgpuweb,errorKind=http,statusCode=403,timeout=false,retriable=false; "
                    + "offering=xgpuwebf2p,errorKind=network,statusCode=none,timeout=true,retriable=true"
            ]
        )

        let payload = CloudLibraryDiagnostics.errorPayload(error)

        XCTAssertEqual(payload["errorKind"], .string("xgpuwebf2p"))
        XCTAssertEqual(payload["statusCode"], .integer(403))
        XCTAssertEqual(payload["timeout"], .bool(true))
        XCTAssertEqual(payload["retriable"], .bool(true))
        XCTAssertEqual(payload["offerings"], .string("xgpuweb,xgpuwebf2p"))
        XCTAssertEqual(payload["forceRegionApplied"], .bool(false))
    }

    func testIOSRuntimeTraceWriterEmitsSchemaV3AndRedactsPayload() async throws {
        let rootDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: rootDirectory) }
        let writer = IOSRuntimeTraceWriter(
            rootDirectory: rootDirectory,
            profile: .dev,
            launchSessionID: "launch-session"
        )

        writer.record(
            IOSRuntimeTraceDraft(
                category: .event,
                domain: "cloud-library",
                event: "catalogRefreshStarted",
                payload: [
                    "generation": .integer(3),
                    "refreshToken": .string("secret-refresh-token"),
                    "hasRefreshToken": .bool(true),
                    "accountID": .string("123456789"),
                    "message": .string("request https://example.invalid/path?token=secret"),
                ],
                dimension: .network,
                importance: .key,
                operationID: "operation-1"
            )
        )
        writer.record(
            IOSRuntimeTraceDraft(
                category: .state,
                domain: "cloud-library",
                event: "catalogRefreshCommitted",
                payload: ["games": .integer(1_888)],
                dimension: .core,
                importance: .essential,
                operationID: "operation-1"
            )
        )

        await writer.flush()
        let envelopes = try await traceEnvelopes(from: writer)
        let events = envelopes.filter { $0.domain == "cloud-library" }

        XCTAssertEqual(events.count, 2)
        XCTAssertEqual(events.map(\.seq), events.map(\.seq).sorted())
        XCTAssertTrue(events.allSatisfy { $0.schemaVersion == 3 })
        XCTAssertTrue(events.allSatisfy { $0.sessionId == "launch-session" })
        XCTAssertEqual(events.first?.payload["refreshToken"], .string("<redacted>"))
        XCTAssertEqual(events.first?.payload["hasRefreshToken"], .bool(true))
        XCTAssertEqual(events.first?.payload["accountID"], .string("<redacted>"))
        XCTAssertEqual(events.first?.payload["message"], .string("request <url>"))
        XCTAssertEqual(events.first?.payload["operationId"], .string("operation-1"))
        XCTAssertEqual(events.first?.payload["platform"], .string("ios"))
    }

    func testIOSRuntimeTraceProductionFiltersDebugRows() async throws {
        let rootDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: rootDirectory) }
        let writer = IOSRuntimeTraceWriter(
            rootDirectory: rootDirectory,
            profile: .production
        )

        writer.record(
            IOSRuntimeTraceDraft(
                category: .event,
                domain: "test",
                event: "debugRow",
                payload: [:],
                dimension: .core,
                importance: .debug,
                operationID: nil
            )
        )
        writer.record(
            IOSRuntimeTraceDraft(
                category: .state,
                domain: "test",
                event: "keyRow",
                payload: [:],
                dimension: .core,
                importance: .key,
                operationID: nil
            )
        )

        await writer.flush()
        let envelopes = try await traceEnvelopes(from: writer)

        XCTAssertFalse(envelopes.contains { $0.event == "debugRow" })
        XCTAssertTrue(envelopes.contains { $0.event == "keyRow" })
    }

    func testIOSRuntimeTraceProfileBudgetsStayBounded() {
        XCTAssertEqual(
            IOSRuntimeTracePolicy.budget(for: .production),
            IOSRuntimeTraceBudget(maxFileBytes: 8 * 1_024 * 1_024, maxFiles: 4)
        )
        XCTAssertEqual(
            IOSRuntimeTracePolicy.budget(for: .dev),
            IOSRuntimeTraceBudget(maxFileBytes: 32 * 1_024 * 1_024, maxFiles: 6)
        )
        XCTAssertEqual(
            IOSRuntimeTracePolicy.budget(for: .off),
            IOSRuntimeTraceBudget(maxFileBytes: 0, maxFiles: 0)
        )
    }

    func testIOSRuntimeTraceRotatesAndPrunesFiles() async throws {
        let rootDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: rootDirectory) }
        let writer = IOSRuntimeTraceWriter(
            rootDirectory: rootDirectory,
            profile: .dev,
            budgetOverride: IOSRuntimeTraceBudget(maxFileBytes: 900, maxFiles: 2)
        )

        for index in 0..<24 {
            writer.record(
                IOSRuntimeTraceDraft(
                    category: .snapshot,
                    domain: "rotation-test",
                    event: "sample",
                    payload: [
                        "index": .integer(Int64(index)),
                        "content": .string(String(repeating: "x", count: 160)),
                    ],
                    dimension: .core,
                    importance: .key,
                    operationID: nil
                )
            )
        }

        await writer.flush()
        let files = await writer.traceFiles()
        let envelopes = try await traceEnvelopes(from: writer)
        let fileSizes = try files.map { file in
            try XCTUnwrap(
                file.resourceValues(forKeys: [.fileSizeKey]).fileSize
            )
        }

        XCTAssertEqual(files.count, 2)
        XCTAssertTrue(fileSizes.allSatisfy { $0 <= 900 })
        XCTAssertFalse(envelopes.isEmpty)
        XCTAssertEqual(envelopes.map(\.seq), envelopes.map(\.seq).sorted())
        XCTAssertEqual(Set(envelopes.map(\.seq)).count, envelopes.count)
        XCTAssertTrue(envelopes.contains { $0.event == "fileOpened" })
    }

    func testCloudCatalogSnapshotRepositoryRoundTripsBaseAndOverlay() async throws {
        let rootDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: rootDirectory) }

        let scope = CloudCatalogScope(
            accountID: "xid",
            regionHost: "region.example.com",
            language: "zh-CN",
            market: "CN"
        )
        let baseUpdatedAt = Date(timeIntervalSince1970: 1_000_000)
        let overlayUpdatedAt = Date(timeIntervalSince1970: 1_000_500)
        let preferredImageURL = URL(string: "https://example.invalid/success.jpg")!
        let game = CloudLibraryGame(
            productID: "P123",
            streamTitleID: "stream-P123",
            xboxTitleID: "123",
            name: "Forza Horizon",
            publisherName: "Xbox Game Studios",
            description: "Open-world racing",
            tileURL: URL(string: "https://example.invalid/tile.jpg"),
            posterURL: URL(string: "https://example.invalid/poster.jpg"),
            heroURL: URL(string: "https://example.invalid/hero.jpg"),
            artworkURL: URL(string: "https://example.invalid/artwork.jpg"),
            categories: ["Racing", "Open World"],
            supportedInputTypes: ["Controller", "Touch"],
            hasEntitlement: true,
            isRecentlyPlayed: true,
            isNew: false,
            lastPlayedAt: Date(timeIntervalSince1970: 999_900),
            playtimeMinutes: 725,
            achievementProgress: AchievementProgress(
                unlockedCount: 12,
                totalCount: 50,
                earnedGamerscore: 240,
                totalGamerscore: 1_000,
                percentage: 24
            )
        )
        let snapshot = CloudCatalogSnapshot(
            scope: scope,
            games: [game],
            baseUpdatedAt: baseUpdatedAt,
            overlayUpdatedAt: overlayUpdatedAt,
            successfulImageURLs: [game.productID: preferredImageURL]
        )
        let repository = CloudCatalogSnapshotRepository(rootDirectory: rootDirectory)

        try await repository.save(snapshot)
        let restored = try await repository.load(scope: scope)

        XCTAssertEqual(restored, snapshot)
        XCTAssertEqual(restored?.cacheState(at: overlayUpdatedAt), .fresh)

        let filenames = try FileManager.default.contentsOfDirectory(
            at: rootDirectory,
            includingPropertiesForKeys: nil
        ).map(\.lastPathComponent)
        XCTAssertEqual(filenames.filter { $0.hasPrefix("base-v1-") }.count, 1)
        XCTAssertEqual(filenames.filter { $0.hasPrefix("overlay-v1-") }.count, 1)
    }

    func testCloudCatalogSnapshotRepositoryClearsAccountOverlayAndKeepsBase() async throws {
        let rootDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: rootDirectory) }

        let scope = CloudCatalogScope(
            accountID: "xid",
            regionHost: "region.example.com",
            language: "zh-CN",
            market: "CN"
        )
        let snapshot = CloudCatalogSnapshot(
            scope: scope,
            games: [makeCloudGame(id: "P123")],
            baseUpdatedAt: .now,
            overlayUpdatedAt: .now
        )
        let repository = CloudCatalogSnapshotRepository(rootDirectory: rootDirectory)

        try await repository.save(snapshot)
        try await repository.clearOverlay(accountID: scope.accountID)

        let restored = try await repository.load(scope: scope)
        XCTAssertNil(restored)
        let filenames = try FileManager.default.contentsOfDirectory(
            at: rootDirectory,
            includingPropertiesForKeys: nil
        ).map(\.lastPathComponent)
        XCTAssertEqual(filenames.filter { $0.hasPrefix("base-v1-") }.count, 1)
        XCTAssertTrue(filenames.allSatisfy { !$0.hasPrefix("overlay-v1-") })
    }

    func testLibraryPresentationKeepsLargeCatalogProgressiveWindowsBounded() {
        let games = (0..<1_000).map { index in
            makeCloudGame(
                id: String(format: "P%04d", index),
                isRecentlyPlayed: index < 25,
                isNew: index < 50
            )
        }
        let collections = LibraryPresentation.collections(fromCloudGames: games)

        XCTAssertEqual(collections.first { $0.kind == .recent }?.games.count, 25)
        XCTAssertEqual(collections.first { $0.kind == .newlyAdded }?.games.count, 50)
        XCTAssertEqual(collections.first { $0.kind == .all }?.games.count, 1_000)
        XCTAssertEqual(collections.first { $0.kind == .all }?.homeGames.count, 8)
        XCTAssertEqual(LibraryPresentation.heroGames(fromCloudGames: games).count, 5)
        XCTAssertEqual(LibraryLayoutMetrics.collectionPageSize, 24)
    }

    @MainActor
    func testCloudLibraryStoreCoalescesConcurrentRefreshes() async {
        let scope = CloudCatalogScope(
            accountID: "xid",
            regionHost: "region.example.com",
            language: "zh-CN",
            market: "CN"
        )
        let client = MockXboxCloudDataClient(
            snapshot: RemoteCloudCatalogSnapshot(
                games: (0..<1_000).map { makeCloudGame(id: "P\($0)") },
                scope: scope,
                fetchedAt: .now,
                failedHydrationChunks: 0,
                pendingHydrationProductIDs: []
            )
        )
        let store = CloudLibraryStore(
            client: client,
            repository: InMemoryCloudCatalogSnapshotStore()
        )
        let access = PreparedCloudAccess(
            authSession: AuthSession(
                refreshToken: "refresh",
                seedJson: "seed",
                webTokenJson: "web",
                appLevel: 2
            ),
            handle: "handle",
            accountID: "xid",
            regionHost: "region.example.com"
        )

        async let first: Void = store.refresh(reason: .pullToRefresh) { access }
        async let second: Void = store.refresh(reason: .manualRetry) { access }
        async let third: Void = store.refresh(reason: .pageEnter) { access }
        _ = await (first, second, third)

        let requestCount = await client.catalogRequestCount()
        XCTAssertEqual(requestCount, 1)
        XCTAssertEqual(store.games.count, 1_000)
        XCTAssertEqual(store.phase, .loaded)
    }

    @MainActor
    func testRestoreWithoutStoredSessionShowsSignedOutState() async {
        let store = AuthStore(
            client: MockXboxAuthClient(),
            keychain: InMemoryAuthSessionStore(),
            webAuthentication: MockWebAuthentication()
        )

        await store.restore()

        XCTAssertEqual(store.phase, .signedOut)
        XCTAssertFalse(store.isSignedIn)
    }

    @MainActor
    func testRestoreRenewsSessionAndLoadsProfile() async {
        let original = StoredAuthSession(
            refreshToken: "old-refresh",
            seedJSON: "seed",
            webTokenJSON: "old-web-token",
            appLevel: 1
        )
        let keychain = InMemoryAuthSessionStore(session: original)
        let client = MockXboxAuthClient(
            renewedSession: AuthSession(
                refreshToken: "new-refresh",
                seedJson: "seed",
                webTokenJson: "new-web-token",
                appLevel: 2
            ),
            profile: Self.profile
        )
        let store = AuthStore(
            client: client,
            keychain: keychain,
            webAuthentication: MockWebAuthentication()
        )

        await store.restore()

        let savedSession = await keychain.currentSession()

        XCTAssertEqual(store.phase, .signedIn)
        XCTAssertEqual(store.profile, Self.profile)
        XCTAssertEqual(store.session?.refreshToken, "new-refresh")
        XCTAssertEqual(savedSession?.refreshToken, "new-refresh")
    }

    @MainActor
    func testRestoreUsesConfiguredCloudRegionForSessionRenewal() async {
        let recorder = RegionRoutingRecorder()
        let client = MockXboxAuthClient(recorder: recorder)
        let settings = MockCloudRegionSettings(preset: .japan)
        let store = AuthStore(
            settings: settings,
            client: client,
            keychain: InMemoryAuthSessionStore(session: Self.storedSession),
            webAuthentication: MockWebAuthentication()
        )

        await store.restore()

        let renewRegionIP = await recorder.lastRenewRegionIP()
        XCTAssertEqual(renewRegionIP, "210.131.113.123")
    }

    @MainActor
    func testInteractiveLoginPersistsSessionAndProfile() async {
        let keychain = InMemoryAuthSessionStore()
        let client = MockXboxAuthClient(
            finishedSession: AuthSession(
                refreshToken: "refresh",
                seedJson: "seed",
                webTokenJson: "web-token",
                appLevel: 2
            ),
            profile: Self.profile
        )
        let webAuthentication = MockWebAuthentication(
            callbackURL: URL(string: "ms-xal-000000004c20a908://auth/?code=code")!
        )
        let store = AuthStore(
            settings: MockCloudRegionSettings(
                preset: .default,
                usesEphemeralLoginSession: true
            ),
            client: client,
            keychain: keychain,
            webAuthentication: webAuthentication
        )

        await store.restore()
        await store.signIn()

        let savedSession = await keychain.currentSession()

        XCTAssertEqual(store.phase, .signedIn)
        XCTAssertEqual(store.profile, Self.profile)
        XCTAssertEqual(savedSession?.webTokenJSON, "web-token")
        XCTAssertEqual(webAuthentication.lastPrefersEphemeralSession, true)
    }

    @MainActor
    func testXboxDataStoreMergesPlaytimeIntoGameLibrary() async {
        let store = XboxDataStore(
            client: MockXboxDataClient(
                games: [Self.game],
                playtimes: [TitlePlaytime(titleID: Self.game.titleID, minutes: 725)]
            )
        )

        await store.sync(session: Self.storedSession)

        XCTAssertEqual(store.libraryPhase, .loaded)
        XCTAssertEqual(store.games.count, 1)
        XCTAssertEqual(store.games.first?.playtimeMinutes, 725)
        XCTAssertNil(store.libraryErrorMessage)
    }

    @MainActor
    func testXboxDataStoreClearsContentAfterSignOut() async {
        let store = XboxDataStore(client: MockXboxDataClient(games: [Self.game]))
        await store.sync(session: Self.storedSession)

        await store.sync(session: nil)

        XCTAssertEqual(store.libraryPhase, .idle)
        XCTAssertTrue(store.games.isEmpty)
        XCTAssertTrue(store.achievements(for: Self.game.titleID).isEmpty)
    }

    @MainActor
    func testXboxDataStoreKeepsLibraryWhenPlaytimeFails() async {
        let store = XboxDataStore(
            client: MockXboxDataClient(games: [Self.game], failsPlaytime: true)
        )

        await store.sync(session: Self.storedSession)

        XCTAssertEqual(store.libraryPhase, .loaded)
        XCTAssertEqual(store.games, [Self.game])
        XCTAssertEqual(store.libraryErrorMessage, "游戏已载入，游玩时长暂时无法更新")
    }

    @MainActor
    func testXboxDataStoreCachesAchievementsUntilForcedRefresh() async {
        let recorder = XboxDataClientRecorder()
        let client = MockXboxDataClient(
            games: [Self.game],
            achievements: [Self.achievement],
            recorder: recorder
        )
        let store = XboxDataStore(client: client)
        await store.sync(session: Self.storedSession)

        await store.loadAchievements(for: Self.game)
        await store.loadAchievements(for: Self.game)
        let cachedRequestCount = await recorder.achievementRequestCount()
        XCTAssertEqual(cachedRequestCount, 1)
        XCTAssertEqual(store.achievements(for: Self.game.titleID), [Self.achievement])

        await store.loadAchievements(for: Self.game, force: true)
        let refreshedRequestCount = await recorder.achievementRequestCount()
        XCTAssertEqual(refreshedRequestCount, 2)
    }

    private static let profile = XboxProfile(
        xuid: "123",
        gamertag: "Player",
        displayName: "Player One",
        gamerScore: "1000",
        displayPictureUrl: "https://example.invalid/avatar.png",
        presenceState: "Online",
        presenceDevice: "Xbox Series X",
        currentTitleName: "Halo Infinite",
        richPresence: "多人游戏大厅",
        followersCount: 24,
        followingCount: 12,
        friendCount: 48
    )

    private static let storedSession = StoredAuthSession(
        refreshToken: "refresh",
        seedJSON: "seed",
        webTokenJSON: "web-token",
        appLevel: 0
    )

    private static let game = GameSummary(
        id: "1292135258",
        titleID: "1292135258",
        name: "Halo Infinite",
        artworkURL: URL(string: "https://example.invalid/box.png"),
        playtimeMinutes: nil,
        achievementProgress: AchievementProgress(
            unlockedCount: 10,
            totalCount: 50,
            earnedGamerscore: 200,
            totalGamerscore: 1000,
            percentage: 20
        )
    )

    private static let achievement = AchievementSummary(
        id: "1",
        titleID: "1292135258",
        name: "First Steps",
        description: "Complete the tutorial",
        imageURL: URL(string: "https://example.invalid/icon.png"),
        isSecret: false,
        isUnlocked: true,
        gamerscore: 25,
        progressPercentage: 100,
        unlockedAt: nil
    )

    private func makeGame(
        id: String,
        name: String,
        lastPlayedAt: Date? = nil,
        playtimeMinutes: Int? = nil,
        percentage: Int? = nil,
        earnedGamerscore: Int = 0
    ) -> GameSummary {
        GameSummary(
            id: id,
            titleID: id,
            name: name,
            artworkURL: nil,
            lastPlayedAt: lastPlayedAt,
            playtimeMinutes: playtimeMinutes,
            achievementProgress: percentage.map { percentage in
                AchievementProgress(
                    unlockedCount: 0,
                    totalCount: 0,
                    earnedGamerscore: earnedGamerscore,
                    totalGamerscore: 1_000,
                    percentage: percentage
                )
            }
        )
    }

    private func makeCloudGame(
        id: String,
        heroURL: URL? = nil,
        posterURL: URL? = nil,
        tileURL: URL? = nil,
        artworkURL: URL? = nil,
        isRecentlyPlayed: Bool = false,
        isNew: Bool = false,
        lastPlayedAt: Date? = nil
    ) -> CloudLibraryGame {
        CloudLibraryGame(
            productID: id,
            streamTitleID: "stream-\(id)",
            xboxTitleID: id,
            name: "Game \(id)",
            publisherName: "Xbox Game Studios",
            description: "Description",
            tileURL: tileURL,
            posterURL: posterURL,
            heroURL: heroURL,
            artworkURL: artworkURL,
            categories: ["Action"],
            supportedInputTypes: ["Controller"],
            hasEntitlement: true,
            isRecentlyPlayed: isRecentlyPlayed,
            isNew: isNew,
            lastPlayedAt: lastPlayedAt
                ?? (isRecentlyPlayed ? Date(timeIntervalSince1970: 1_000) : nil),
            playtimeMinutes: nil,
            achievementProgress: nil
        )
    }

    private func traceEnvelopes(
        from writer: IOSRuntimeTraceWriter
    ) async throws -> [IOSRuntimeTraceEnvelope] {
        let decoder = JSONDecoder()
        let files = await writer.traceFiles()
        return try files.flatMap { file in
            try String(contentsOf: file, encoding: .utf8)
                .split(separator: "\n")
                .map { line in
                    try decoder.decode(
                        IOSRuntimeTraceEnvelope.self,
                        from: Data(line.utf8)
                    )
                }
        }
    }
}

private actor InMemoryCloudCatalogSnapshotStore: CloudCatalogSnapshotStoring {
    private var snapshots: [CloudCatalogScope: CloudCatalogSnapshot] = [:]

    func load(scope: CloudCatalogScope) async throws -> CloudCatalogSnapshot? {
        snapshots[scope]
    }

    func save(_ snapshot: CloudCatalogSnapshot) async throws {
        snapshots[snapshot.scope] = snapshot
    }

    func clearOverlay(accountID: String) async throws {
        snapshots = snapshots.filter { $0.key.accountID != accountID }
    }
}

private actor MockXboxCloudDataClient: XboxCloudDataClient {
    private let snapshot: RemoteCloudCatalogSnapshot
    private var catalogRequests = 0

    init(snapshot: RemoteCloudCatalogSnapshot) {
        self.snapshot = snapshot
    }

    func prepareAccess(
        refreshToken _: String,
        seedJSON _: String,
        forceRegionIP _: String
    ) async throws -> PreparedCloudAccess {
        PreparedCloudAccess(
            authSession: AuthSession(
                refreshToken: "refresh",
                seedJson: "seed",
                webTokenJson: "web",
                appLevel: 2
            ),
            handle: "handle",
            accountID: snapshot.scope.accountID,
            regionHost: snapshot.scope.regionHost
        )
    }

    func loadCatalog(
        accessHandle _: String,
        market _: String,
        language _: String
    ) async throws -> RemoteCloudCatalogSnapshot {
        catalogRequests += 1
        try await Task.sleep(for: .milliseconds(40))
        return snapshot
    }

    func loadMetadataPage(
        accessHandle _: String,
        market _: String,
        language _: String,
        productIDs _: [String]
    ) async throws -> [CloudCatalogMetadata] {
        []
    }

    func releaseAccess(handle _: String) async {}

    func catalogRequestCount() -> Int {
        catalogRequests
    }
}

private actor InMemoryAuthSessionStore: AuthSessionStoring {
    private var session: StoredAuthSession?

    init(session: StoredAuthSession? = nil) {
        self.session = session
    }

    func load() async throws -> StoredAuthSession? {
        session
    }

    func save(_ session: StoredAuthSession) async throws {
        self.session = session
    }

    func delete() async throws {
        session = nil
    }

    func currentSession() -> StoredAuthSession? {
        session
    }
}

private struct MockXboxAuthClient: XboxAuthClient {
    let renewedSession: AuthSession
    let finishedSession: AuthSession
    let profile: XboxProfile
    let recorder: RegionRoutingRecorder?

    init(
        renewedSession: AuthSession = AuthSession(
            refreshToken: "refresh",
            seedJson: "seed",
            webTokenJson: "web-token",
            appLevel: 1
        ),
        finishedSession: AuthSession = AuthSession(
            refreshToken: "refresh",
            seedJson: "seed",
            webTokenJson: "web-token",
            appLevel: 1
        ),
        profile: XboxProfile = XboxProfile(
            xuid: nil,
            gamertag: "Player",
            displayName: "Player",
            gamerScore: "0",
            displayPictureUrl: "",
            presenceState: nil,
            presenceDevice: nil,
            currentTitleName: nil,
            richPresence: nil,
            followersCount: nil,
            followingCount: nil,
            friendCount: nil
        ),
        recorder: RegionRoutingRecorder? = nil
    ) {
        self.renewedSession = renewedSession
        self.finishedSession = finishedSession
        self.profile = profile
        self.recorder = recorder
    }

    func beginLogin() async throws -> LoginStartResult {
        LoginStartResult(
            authorizationUrl: "https://login.live.com/oauth20_authorize.srf",
            state: "state",
            pendingJson: "pending",
            seedJson: "seed"
        )
    }

    func finishLogin(
        callbackURL _: URL,
        pendingJSON _: String,
        seedJSON _: String,
        forceRegionIP: String
    ) async throws -> AuthSession {
        await recorder?.recordFinish(forceRegionIP)
        return finishedSession
    }

    func renewLogin(
        refreshToken _: String,
        seedJSON _: String,
        forceRegionIP: String
    ) async throws -> AuthSession {
        await recorder?.recordRenew(forceRegionIP)
        return renewedSession
    }

    func loadProfile(webTokenJSON _: String) async throws -> XboxProfile {
        profile
    }
}

private actor RegionRoutingRecorder {
    private var finishRegionIP: String?
    private var renewRegionIP: String?

    func recordFinish(_ value: String) {
        finishRegionIP = value
    }

    func recordRenew(_ value: String) {
        renewRegionIP = value
    }

    func lastRenewRegionIP() -> String? {
        renewRegionIP
    }
}

@MainActor
private final class MockCloudRegionSettings: CloudRegionSettingsProviding {
    let cloudRegionPreset: CloudRegionPreset
    let usesEphemeralLoginSession: Bool

    init(preset: CloudRegionPreset, usesEphemeralLoginSession: Bool = false) {
        cloudRegionPreset = preset
        self.usesEphemeralLoginSession = usesEphemeralLoginSession
    }
}

private struct MockXboxDataClient: XboxDataClient {
    let games: [GameSummary]
    let playtimes: [TitlePlaytime]
    let achievements: [AchievementSummary]
    let recorder: XboxDataClientRecorder?
    let failsPlaytime: Bool

    init(
        games: [GameSummary] = [],
        playtimes: [TitlePlaytime] = [],
        achievements: [AchievementSummary] = [],
        recorder: XboxDataClientRecorder? = nil,
        failsPlaytime: Bool = false
    ) {
        self.games = games
        self.playtimes = playtimes
        self.achievements = achievements
        self.recorder = recorder
        self.failsPlaytime = failsPlaytime
    }

    func loadGameLibrary(webTokenJSON _: String) async throws -> [GameSummary] {
        games
    }

    func loadPlaytimes(
        webTokenJSON _: String,
        titleIDs _: [String]
    ) async throws -> [TitlePlaytime] {
        if failsPlaytime {
            throw MockXboxDataError.unavailable
        }
        return playtimes
    }

    func loadAchievements(
        webTokenJSON _: String,
        titleID _: String
    ) async throws -> [AchievementSummary] {
        await recorder?.recordAchievementRequest()
        return achievements
    }
}

private enum MockXboxDataError: Error {
    case unavailable
}

private actor XboxDataClientRecorder {
    private var achievementRequests = 0

    func recordAchievementRequest() {
        achievementRequests += 1
    }

    func achievementRequestCount() -> Int {
        achievementRequests
    }
}

@MainActor
private final class MockWebAuthentication: WebAuthenticating {
    private let callbackURL: URL
    private(set) var lastPrefersEphemeralSession: Bool?

    init(
        callbackURL: URL = URL(string: "ms-xal-000000004c20a908://auth/?code=code")!
    ) {
        self.callbackURL = callbackURL
    }

    func authenticate(
        authorizationURL _: String,
        prefersEphemeralSession: Bool
    ) async throws -> URL {
        lastPrefersEphemeralSession = prefersEphemeralSession
        return callbackURL
    }

    func cancel() {}
}
