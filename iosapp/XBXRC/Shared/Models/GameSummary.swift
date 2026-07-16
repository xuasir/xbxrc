import Foundation

struct GameSummary: Identifiable, Equatable, Hashable, Sendable {
    let id: String
    let titleID: String
    let name: String
    let artworkURL: URL?
    let heroURL: URL?
    let lastPlayedAt: Date?
    let playtimeMinutes: Int?
    let achievementProgress: AchievementProgress?

    init(
        id: String,
        titleID: String,
        name: String,
        artworkURL: URL?,
        heroURL: URL? = nil,
        lastPlayedAt: Date? = nil,
        playtimeMinutes: Int?,
        achievementProgress: AchievementProgress?
    ) {
        self.id = id
        self.titleID = titleID
        self.name = name
        self.artworkURL = artworkURL
        self.heroURL = heroURL
        self.lastPlayedAt = lastPlayedAt
        self.playtimeMinutes = playtimeMinutes
        self.achievementProgress = achievementProgress
    }
}

struct AchievementProgress: Equatable, Hashable, Sendable {
    let unlockedCount: Int
    let totalCount: Int
    let earnedGamerscore: Int
    let totalGamerscore: Int
    let percentage: Int

    init(
        unlockedCount: Int,
        totalCount: Int,
        earnedGamerscore: Int,
        totalGamerscore: Int,
        percentage: Int? = nil
    ) {
        self.unlockedCount = unlockedCount
        self.totalCount = totalCount
        self.earnedGamerscore = earnedGamerscore
        self.totalGamerscore = totalGamerscore
        self.percentage = percentage ?? Self.resolvePercentage(
            earned: earnedGamerscore,
            total: totalGamerscore
        )
    }

    private static func resolvePercentage(earned: Int, total: Int) -> Int {
        guard total > 0 else {
            return 0
        }
        return min(100, max(0, Int((Double(earned) / Double(total) * 100).rounded())))
    }
}

struct AchievementSummary: Identifiable, Equatable, Hashable, Sendable {
    let id: String
    let titleID: String
    let name: String
    let description: String
    let imageURL: URL?
    let isSecret: Bool
    let isUnlocked: Bool
    let gamerscore: Int
    let progressPercentage: Int?
    let unlockedAt: Date?
}

enum DataLoadPhase: Equatable, Sendable {
    case idle
    case loading
    case loaded
    case failed
}
