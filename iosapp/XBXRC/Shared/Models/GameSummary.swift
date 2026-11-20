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
    let description: String?
    let productID: String?
    let scid: String?

    init(
        id: String,
        titleID: String,
        name: String,
        artworkURL: URL?,
        heroURL: URL? = nil,
        lastPlayedAt: Date? = nil,
        playtimeMinutes: Int?,
        achievementProgress: AchievementProgress?,
        description: String? = nil, productID: String? = nil, scid: String? = nil
    ) {
        self.id = id
        self.titleID = titleID
        self.name = name
        self.artworkURL = artworkURL
        self.heroURL = heroURL
        self.lastPlayedAt = lastPlayedAt
        self.playtimeMinutes = playtimeMinutes
        self.achievementProgress = achievementProgress
        self.description = description
        self.productID = productID
        self.scid = scid
    }

    func replacingPlaytime(_ minutes: Int?) -> GameSummary {
        GameSummary(
            id: id, titleID: titleID, name: name, artworkURL: artworkURL,
            heroURL: heroURL, lastPlayedAt: lastPlayedAt, playtimeMinutes: minutes,
            achievementProgress: achievementProgress, description: description,
            productID: productID, scid: scid)
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
        self.percentage =
            percentage
            ?? Self.resolvePercentage(
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
    var scid: String? = nil
    var lockedDescription: String? = nil
    var requirements: [AchievementRequirementSummary] = []
    var queryIdentity: String { "\(scid ?? ""):\(id)" }
}

struct AchievementRequirementSummary: Equatable, Hashable, Sendable {
    let id: String
    let current: String?
    let target: String?
}

enum DataLoadPhase: Equatable, Sendable {
    case idle
    case loading
    case loaded
    case failed
}
