import Foundation

extension RustXboxDataClient {
    func loadPlayerProfile(webTokenJSON: String, targetID: String) async throws -> XboxFriendSummary
    {
        let profile = try await fetchPlayerProfile(webTokenJson: webTokenJSON, targetXuid: targetID)
        return XboxFriendSummary(
            id: profile.id, displayName: profile.displayName, gamertag: profile.gamertag,
            avatarURL: XboxImageURL.resolve(profile.avatarUrl), gamerscore: profile.gamerscore,
            presenceState: profile.presenceState, presenceText: profile.presenceText,
            currentTitleID: profile.currentTitleId)
    }

    func loadHistoryPage(webTokenJSON: String, targetID: String?, continuation: String?)
        async throws -> XboxHistoryPage
    {
        let page = try await fetchGameHistoryPage(
            webTokenJson: webTokenJSON, targetXuid: targetID, continuation: continuation)
        return XboxHistoryPage(items: page.items.map(personalGame), continuation: page.continuation)
    }

    func loadLibraryGame(webTokenJSON: String, titleID: String, targetID: String?) async throws
        -> GameSummary?
    {
        try await fetchLibraryGame(
            webTokenJson: webTokenJSON, titleId: titleID, targetXuid: targetID
        ).map(personalGame)
    }

    func loadPlayerPlaytimes(webTokenJSON: String, titleIDs: [String], targetID: String?)
        async throws -> XboxTitlePlaytimeSnapshot
    {
        let result = try await fetchPlayerPlaytimes(
            webTokenJson: webTokenJSON, targetXuid: targetID, titleIds: titleIDs)
        return XboxTitlePlaytimeSnapshot(
            items: result.items.map {
                TitlePlaytime(titleID: $0.titleId, minutes: $0.minutes.flatMap(Int.init(exactly:)))
            },
            failedTitleIDs: Set(result.failedTitleIds),
            failures: Dictionary(
                result.failures.map { ($0.source, SectionRequestFailure($0.failure)) },
                uniquingKeysWith: { first, _ in first }))
    }

    func loadAchievementPage(
        webTokenJSON: String, titleID: String, targetID: String?, locale: String,
        continuation: String?
    ) async throws -> XboxAchievementsPage {
        let page = try await fetchAchievementPage(
            webTokenJson: webTokenJSON, titleId: titleID, targetXuid: targetID,
            locale: locale, continuation: continuation)
        return XboxAchievementsPage(
            items: page.items.map { item in
                AchievementSummary(
                    id: item.id, titleID: item.titleId, name: item.name,
                    description: item.description.isEmpty
                        ? item.lockedDescription : item.description,
                    imageURL: XboxImageURL.resolve(item.imageUrl), isSecret: item.isSecret,
                    isUnlocked: item.isUnlocked, gamerscore: Int(item.gamerscore),
                    progressPercentage: item.progressPercentage.map(Int.init),
                    unlockedAt: personalDate(item.unlockedAt),
                    scid: item.scid, lockedDescription: item.lockedDescription,
                    requirements: item.requirements.map {
                        AchievementRequirementSummary(
                            id: $0.id, current: $0.current, target: $0.target)
                    })
            }, continuation: page.continuation)
    }
}

// 默认实现用于现有 XCTest 替身；生产客户端全部调用 Rust 的目标/分页接口。
extension XboxDataClient {
    func loadPlayerProfile(webTokenJSON: String, targetID: String) async throws -> XboxFriendSummary
    {
        throw XboxNativeQueryError.unavailable
    }
    func loadHistoryPage(webTokenJSON: String, targetID: String?, continuation: String?)
        async throws -> XboxHistoryPage
    {
        guard targetID == nil, continuation == nil else { throw XboxNativeQueryError.unavailable }
        return XboxHistoryPage(
            items: try await loadGameLibrary(webTokenJSON: webTokenJSON), continuation: nil)
    }
    func loadLibraryGame(webTokenJSON: String, titleID: String, targetID: String?) async throws
        -> GameSummary?
    {
        guard targetID == nil else { throw XboxNativeQueryError.unavailable }
        return try await loadGameLibrary(webTokenJSON: webTokenJSON).first { $0.titleID == titleID }
    }
    func loadPlayerPlaytimes(webTokenJSON: String, titleIDs: [String], targetID: String?)
        async throws -> XboxTitlePlaytimeSnapshot
    {
        guard targetID == nil else { throw XboxNativeQueryError.unavailable }
        return XboxTitlePlaytimeSnapshot(
            items: try await loadPlaytimes(webTokenJSON: webTokenJSON, titleIDs: titleIDs))
    }
    func loadAchievementPage(
        webTokenJSON: String, titleID: String, targetID: String?, locale: String,
        continuation: String?
    ) async throws -> XboxAchievementsPage {
        guard targetID == nil, continuation == nil else { throw XboxNativeQueryError.unavailable }
        return XboxAchievementsPage(
            items: try await loadAchievements(
                webTokenJSON: webTokenJSON, titleID: titleID, locale: locale), continuation: nil)
    }
}

private func personalGame(_ game: XboxLibraryGame) -> GameSummary {
    GameSummary(
        id: game.titleId, titleID: game.titleId, name: game.name,
        artworkURL: XboxImageURL.resolve(game.artworkUrl),
        lastPlayedAt: personalDate(game.lastPlayedAt),
        playtimeMinutes: nil,
        achievementProgress: game.achievementProgress.map {
            AchievementProgress(
                unlockedCount: Int($0.unlockedCount), totalCount: Int($0.totalCount),
                earnedGamerscore: Int($0.earnedGamerscore),
                totalGamerscore: Int($0.totalGamerscore), percentage: Int($0.percentage))
        }, description: game.description, productID: game.productId, scid: game.scid)
}

private func personalDate(_ value: String?) -> Date? {
    value.flatMap { try? Date($0, strategy: .iso8601) }
}

extension XboxDataClient {
    func loadInstalledSection(
        webTokenJSON: String, consoleID: String, market: String, language: String,
        source: XboxInstalledSource, productIDs: [String]
    ) async throws -> InstalledSectionResult {
        let snapshot = try await loadInstalledLibrary(
            webTokenJSON: webTokenJSON, consoleID: consoleID, market: market, language: language)
        let missing =
            source == .games
            ? snapshot.games == nil
            : source == .storage
                ? snapshot.storage == nil : source == .status ? snapshot.powerState == nil : false
        return InstalledSectionResult(
            snapshot: snapshot, failure: missing ? SectionRequestFailure(kind: .unknown) : nil)
    }
}
