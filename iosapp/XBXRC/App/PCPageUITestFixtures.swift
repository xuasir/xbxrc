#if DEBUG
    import Foundation

    /// 仅由 XCTest 的显式启动参数装配，所有身份、凭据及结果均为固定验证数据。
    enum PCPageFixtures {
        static let session = StoredAuthSession(
            refreshToken: "fixture-refresh", seedJSON: "{}",
            webTokenJSON:
                "{\"DisplayClaims\":{\"xui\":[{\"uhs\":\"fixture\",\"xid\":\"1000\"}]},\"Token\":\"fixture\"}",
            appLevel: 0, cloudAccountID: "1000",
            cloudRegionHost: "uswest.gssv-play-prod.xboxlive.com")
        static var bridgeSession: AuthSession {
            AuthSession(
                refreshToken: session.refreshToken, seedJson: session.seedJSON,
                webTokenJson: session.webTokenJSON, appLevel: 0)
        }
        static let profile = XboxProfile(
            xuid: "1000", gamertag: "移动端验收", displayName: "玩家", gamerScore: "1200",
            displayPictureUrl: "",
            presenceState: "Online", presenceDevice: "Xbox", currentTitleName: "Halo Infinite",
            richPresence: "正在游玩 Halo Infinite",
            followersCount: 12, followingCount: 8, friendCount: 2)
        static let friends = [
            XboxFriendSummary(
                id: "2000", displayName: "好友一", gamertag: "在线好友", avatarURL: nil, gamerscore: "800",
                presenceState: "Online", presenceText: "Halo Infinite", currentTitleID: "1"),
            XboxFriendSummary(
                id: "3000", displayName: "好友二", gamertag: "离线好友", avatarURL: nil, gamerscore: "400",
                presenceState: "Offline", presenceText: "离线"),
        ]
        static let games = [
            GameSummary(
                id: "1", titleID: "1", name: "Halo Infinite", artworkURL: nil,
                lastPlayedAt: Date(timeIntervalSince1970: 1_790_000_000),
                playtimeMinutes: nil,
                achievementProgress: AchievementProgress(
                    unlockedCount: 2, totalCount: 10, earnedGamerscore: 20, totalGamerscore: 100),
                description: "探索光环世界，查看个人成就与游玩记录。", productID: "HALO00000001"),
            GameSummary(
                id: "2", titleID: "2", name: "Forza Horizon 5", artworkURL: nil,
                lastPlayedAt: Date(timeIntervalSince1970: 1_789_000_000),
                playtimeMinutes: nil,
                achievementProgress: AchievementProgress(
                    unlockedCount: 10, totalCount: 10, earnedGamerscore: 100, totalGamerscore: 100),
                description: String(
                    repeating: "驾驶各式赛车探索墨西哥的海岸、雨林与山地，参与赛事并与好友共同完成挑战。\n\n",
                    count: 6),
                productID: "FORZA0000001"),
            GameSummary(
                id: "3", titleID: "3", name: "Sea of Thieves", artworkURL: nil,
                playtimeMinutes: nil, achievementProgress: nil, productID: "SEA000000001"),
        ]
        static let host = XboxHostSummary(
            id: "fixture-host", commandID: "fixture-host", streamTargetID: "fixture-host",
            name: "我的 Xbox", consoleType: "Xbox Series X",
            locale: "zh-CN", region: "CN", powerState: "On", remoteManagementEnabled: true,
            consoleStreamingEnabled: true,
            wirelessWarning: false, outOfHomeWarning: false, storageDevices: [])
        static let standbyHost = XboxHostSummary(
            id: "fixture-series-s", commandID: "fixture-series-s",
            streamTargetID: "fixture-series-s",
            name: "卧室 Xbox", consoleType: "XboxSeriesS",
            locale: "zh-CN", region: "CN", powerState: "ConnectedStandby",
            remoteManagementEnabled: true, consoleStreamingEnabled: true,
            wirelessWarning: false, outOfHomeWarning: false,
            storageDevices: [
                XboxHostStorageSummary(
                    id: "internal", name: "内部存储", freeBytes: 16_250_000_000,
                    totalBytes: 512_000_000_000)
            ])
        static let offlineHost = XboxHostSummary(
            id: "fixture-offline", commandID: "fixture-offline", streamTargetID: "fixture-offline",
            name: "书房 Xbox", consoleType: "Xbox Series X",
            locale: "zh-CN", region: "CN", powerState: "Off",
            remoteManagementEnabled: false, consoleStreamingEnabled: false,
            wirelessWarning: false, outOfHomeWarning: false, storageDevices: [])
        static func cloudGames() -> [CloudLibraryGame] {
            games.enumerated().map { index, game in
                CloudLibraryGame(
                    productID: game.productID!, streamTitleID: "stream-\(game.titleID)",
                    xboxTitleID: game.titleID,
                    name: game.name, publisherName: "Xbox Game Studios",
                    description: game.description ?? "Xbox 云游戏",
                    tileURL: nil, posterURL: nil, heroURL: nil, artworkURL: nil,
                    categories: [index == 1 ? "竞速" : "动作"],
                    supportedInputTypes: ["controller", "touch"], hasEntitlement: true,
                    isRecentlyPlayed: index == 0, isNew: index == 1,
                    lastPlayedAt: game.lastPlayedAt, playtimeMinutes: index == 0 ? 150 : 60,
                    achievementProgress: game.achievementProgress,
                    recentRank: index == 0 ? 0 : nil, newRank: index == 1 ? 0 : nil,
                    popularRank: index, leavingRank: index == 2 ? 0 : nil)
            }
        }
        static func achievements(titleID: String, targetID: String?) -> [AchievementSummary] {
            [
                AchievementSummary(
                    id: "secret", titleID: titleID, name: "探索者", description: "发现隐藏区域",
                    imageURL: nil,
                    isSecret: true, isUnlocked: targetID != nil, gamerscore: 10,
                    progressPercentage: targetID == nil ? 20 : 100, unlockedAt: nil,
                    scid: "core", lockedDescription: "发现隐藏区域",
                    requirements: [
                        AchievementRequirementSummary(id: "steps", current: "2", target: "10")
                    ]),
                AchievementSummary(
                    id: "first", titleID: titleID, name: "初次启程", description: "完成第一项挑战",
                    imageURL: nil,
                    isSecret: false, isUnlocked: true, gamerscore: 10, progressPercentage: 100,
                    unlockedAt: Date(timeIntervalSince1970: 1_790_000_000), scid: "core"),
            ]
        }
    }

    actor PCSectionFailureFixture {
        private var calls: [String: Int] = [:]
        func check(_ source: String) throws {
            guard ProcessInfo.processInfo.arguments.contains("--section-errors-ui-test") else {
                return
            }
            calls[source, default: 0] += 1
            if calls[source] == 1 { throw SectionRequestFailure(kind: .timeout) }
        }
    }

    struct PCPageDataClient: XboxDataClient {
        private let errors = PCSectionFailureFixture()
        var includesAllHosts = ProcessInfo.processInfo.arguments.contains("--host-switcher-ui-test")

        func loadPlayerProfile(webTokenJSON: String, targetID: String) async throws
            -> XboxFriendSummary
        {
            PCPageFixtures.friends.first { $0.id == targetID } ?? PCPageFixtures.friends[0]
        }
        func loadHistoryPage(webTokenJSON: String, targetID: String?, continuation: String?)
            async throws -> XboxHistoryPage
        {
            XboxHistoryPage(
                items: continuation == nil
                    ? Array(PCPageFixtures.games.prefix(2)) : [PCPageFixtures.games[2]],
                continuation: continuation == nil ? "page2" : nil)
        }
        func loadLibraryGame(webTokenJSON: String, titleID: String, targetID: String?) async throws
            -> GameSummary?
        {
            PCPageFixtures.games.first { $0.titleID == titleID }
        }
        func loadPlayerPlaytimes(webTokenJSON: String, titleIDs: [String], targetID: String?)
            async throws -> XboxTitlePlaytimeSnapshot
        {
            XboxTitlePlaytimeSnapshot(
                items: titleIDs.map { TitlePlaytime(titleID: $0, minutes: $0 == "1" ? 150 : 60) })
        }
        func loadAchievementPage(
            webTokenJSON: String, titleID: String, targetID: String?, locale: String,
            continuation: String?
        ) async throws -> XboxAchievementsPage {
            XboxAchievementsPage(
                items: PCPageFixtures.achievements(titleID: titleID, targetID: targetID),
                continuation: nil)
        }
        func loadFriends(webTokenJSON: String) async throws -> [XboxFriendSummary] {
            try await errors.check("friends")
            return PCPageFixtures.friends
        }
        func loadMedia(webTokenJSON: String, kind: XboxAlbumKind, continuation: String?)
            async throws -> XboxAlbumPage
        {
            if kind == .screenshot { try await errors.check("screenshot") }
            return XboxAlbumPage(
                items: [
                    XboxAlbumMedia(
                        id: kind.rawValue, kind: kind, titleID: "1", titleName: "Halo Infinite",
                        capturedAt: Date(
                            timeIntervalSince1970: kind == .clip ? 1_790_000_000 : 1_790_000_001),
                        durationSeconds: kind == .clip ? 30 : nil, thumbnailURL: nil, url: nil,
                        expiresAt: nil)
                ], continuation: nil)
        }
        func loadInstalledSection(
            webTokenJSON: String, consoleID: String, market: String, language: String,
            source: XboxInstalledSource, productIDs: [String]
        ) async throws -> InstalledSectionResult {
            if source == .storage { try await errors.check("storage") }
            let snapshot = try await loadInstalledLibrary(
                webTokenJSON: webTokenJSON, consoleID: consoleID,
                market: market, language: language)
            return InstalledSectionResult(snapshot: snapshot)
        }

        func loadInstalledLibrary(
            webTokenJSON: String, consoleID: String, market: String, language: String
        ) async throws -> XboxInstalledLibrarySnapshot {
            XboxInstalledLibrarySnapshot(
                consoleID: consoleID,
                games: PCPageFixtures.games.enumerated().map { index, game in
                    XboxInstalledGameSummary(
                        id: game.id,
                        titleID: index == 1
                            && ProcessInfo.processInfo.arguments.contains(
                                "--installed-product-only-ui-test")
                            ? nil : game.titleID,
                        productID: game.productID,
                        name: game.name, artworkURL: nil,
                        storageDeviceID: index == 2 ? "external" : "internal",
                        sizeBytes: UInt64(100_000_000 * (index + 1)),
                        installedAt: Date(
                            timeIntervalSince1970: 1_780_000_000 + Double(index) * 1000),
                        lastPlayedAt: game.lastPlayedAt)
                },
                storage: [
                    XboxHostStorageSummary(
                        id: "internal", name: "内部存储", freeBytes: 100_000_000,
                        totalBytes: 1_000_000_000),
                    XboxHostStorageSummary(
                        id: "external", name: "扩展存储", freeBytes: 300_000_000,
                        totalBytes: 1_000_000_000),
                ], powerState: "On", notices: [])
        }
        func loadHosts(webTokenJSON: String) async throws -> [XboxHostSummary] {
            includesAllHosts
                ? [PCPageFixtures.host, PCPageFixtures.standbyHost, PCPageFixtures.offlineHost]
                : [PCPageFixtures.host]
        }
        func powerOn(webTokenJSON: String, consoleID: String) async throws -> HostPowerCommandResult
        {
            HostPowerCommandResult(consoleID: consoleID, accepted: true)
        }
        func powerOff(webTokenJSON: String, consoleID: String) async throws
            -> HostPowerCommandResult
        { HostPowerCommandResult(consoleID: consoleID, accepted: true) }
        func loadGameLibrary(webTokenJSON: String) async throws -> [GameSummary] {
            PCPageFixtures.games
        }
        func loadPlaytimes(webTokenJSON: String, titleIDs: [String]) async throws -> [TitlePlaytime]
        {
            titleIDs.map { TitlePlaytime(titleID: $0, minutes: 60) }
        }
        func loadAchievements(webTokenJSON: String, titleID: String, locale: String) async throws
            -> [AchievementSummary]
        {
            PCPageFixtures.achievements(titleID: titleID, targetID: nil)
        }
    }

    struct PCPageCloudClient: XboxCloudDataClient {
        func prepareAccess(
            session: StoredAuthSession, forceRegionIP: String,
            checkpoint: @escaping AuthCredentialCheckpoint,
            authProvider: (any XboxStreamingAuthProvider)?, startup: XboxStreamStartup?
        )
            async throws -> PreparedCloudAccess
        {
            PreparedCloudAccess(
                authSession: PCPageFixtures.bridgeSession, handle: "fixture-access",
                accountID: "1000",
                regionHost: "uswest.gssv-play-prod.xboxlive.com",
                expiresAtMs: UInt64(Date().addingTimeInterval(3600).timeIntervalSince1970 * 1000))
        }
        func prepareHomeAccess(
            session: StoredAuthSession, forceRegionIP: String,
            checkpoint: @escaping AuthCredentialCheckpoint,
            authProvider: (any XboxStreamingAuthProvider)?, startup: XboxStreamStartup?
        )
            async throws -> PreparedHomeAccess
        { throw XboxNativeQueryError.unavailable }
        func loadCatalog(accessHandle: String, market: String, language: String) async throws
            -> RemoteCloudCatalogSnapshot
        {
            let fails = ProcessInfo.processInfo.arguments.contains("--section-errors-ui-test")
            return RemoteCloudCatalogSnapshot(
                games: PCPageFixtures.cloudGames(),
                scope: CloudCatalogScope(
                    accountID: "1000", regionHost: "uswest.gssv-play-prod.xboxlive.com",
                    language: language, market: market),
                fetchedAt: .now, failedHydrationChunks: 0, pendingHydrationProductIDs: [],
                unavailableSources: fails ? ["popular"] : [],
                sourceFailures: fails ? ["popular": SectionRequestFailure(kind: .timeout)] : [:])
        }
        func loadCatalogSource(
            accessHandle: String, source: String, market: String, language: String
        )
            async throws -> [String]
        {
            PCPageFixtures.cloudGames().map(\.productID)
        }
        func loadMetadataPage(
            accessHandle: String, market: String, language: String, productIDs: [String]
        ) async throws -> [CloudCatalogMetadata] { [] }
        func releaseAccess(handle: String) async {}
        func loadProductDetail(productID: String, market: String, language: String) async throws
            -> CloudProductDetails
        {
            let game =
                PCPageFixtures.cloudGames().first { $0.productID == productID }
                ?? PCPageFixtures.cloudGames()[0]
            return CloudProductDetails(
                productID: productID, name: game.name, publisherName: game.publisherName,
                description: game.description ?? "完整云游戏商品资料",
                tileURL: nil, posterURL: nil, heroURL: nil, categories: game.categories,
                developer: "Xbox Game Studios",
                releaseDate: "2025-01-01", rating: "12+", attributes: ["单人游戏", "在线多人"],
                screenshots: [URL(string: "https://example.invalid/fixture-screenshot.png")!],
                platforms: ["Xbox Series X|S"],
                languages: [
                    CloudLanguageSupport(locale: "zh-Hans", interface: 1, audio: 0, subtitles: 1),
                    CloudLanguageSupport(locale: "en-US", interface: 1, audio: 1, subtitles: 1),
                ], accessibility: ["字幕选项", "文本大小"])
        }
        func loadWaitTime(accessHandle: String, streamTitleID: String, serverRegion: String)
            async throws -> UInt64?
        { 0 }
    }

    struct PCPageAuthClient: XboxAuthClient {
        func beginLogin() async throws -> LoginStartResult {
            throw XboxNativeQueryError.unavailable
        }
        func finishLogin(
            callbackURL: URL, pendingJSON: String, seedJSON: String, forceRegionIP: String
        ) async throws -> AuthSession { PCPageFixtures.bridgeSession }
        func renewLogin(
            session: StoredAuthSession, forceRegionIP: String, scope: XboxAuthScope,
            forceCoreRefresh: Bool, rejectedToken: String?,
            checkpoint: @escaping AuthCredentialCheckpoint
        ) async throws
            -> AuthSession
        { PCPageFixtures.bridgeSession }
        func loadProfile(webTokenJSON: String) async throws -> XboxProfile {
            PCPageFixtures.profile
        }
    }

    actor PCPageSessionStore: AuthSessionStoring {
        private var session: StoredAuthSession? = PCPageFixtures.session
        func load() async throws -> StoredAuthSession? { session }
        func save(_ session: StoredAuthSession) async throws { self.session = session }
        func delete() async throws { session = nil }
    }
#endif
