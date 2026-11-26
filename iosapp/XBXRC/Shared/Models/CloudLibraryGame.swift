import Foundation

extension AchievementProgress: Codable {
    private enum CodingKeys: String, CodingKey {
        case unlockedCount
        case totalCount
        case earnedGamerscore
        case totalGamerscore
        case percentage
    }

    init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.init(
            unlockedCount: try container.decode(Int.self, forKey: .unlockedCount),
            totalCount: try container.decode(Int.self, forKey: .totalCount),
            earnedGamerscore: try container.decode(Int.self, forKey: .earnedGamerscore),
            totalGamerscore: try container.decode(Int.self, forKey: .totalGamerscore),
            percentage: try container.decode(Int.self, forKey: .percentage)
        )
    }

    func encode(to encoder: any Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(unlockedCount, forKey: .unlockedCount)
        try container.encode(totalCount, forKey: .totalCount)
        try container.encode(earnedGamerscore, forKey: .earnedGamerscore)
        try container.encode(totalGamerscore, forKey: .totalGamerscore)
        try container.encode(percentage, forKey: .percentage)
    }
}

struct CloudLibraryGame: Codable, Hashable, Identifiable, Sendable {
    let productID: String
    let streamTitleID: String?
    let xboxTitleID: String?
    let name: String
    let publisherName: String
    let description: String
    let tileURL: URL?
    let posterURL: URL?
    let heroURL: URL?
    let artworkURL: URL?
    let categories: [String]
    let supportedInputTypes: [String]
    let hasEntitlement: Bool?
    var isRecentlyPlayed: Bool?
    var isNew: Bool?
    var recentRank: Int?
    var newRank: Int?
    var popularRank: Int?
    var leavingRank: Int?
    let lastPlayedAt: Date?
    let playtimeMinutes: Int?
    let achievementProgress: AchievementProgress?

    var id: String { productID }

    var imageCandidates: [URL] {
        imageCandidates(preferredURL: nil)
    }

    func imageCandidates(preferredURL: URL?) -> [URL] {
        var seen = Set<String>()
        return [preferredURL, heroURL, posterURL, tileURL, artworkURL]
            .compactMap { $0 }
            .filter { seen.insert($0.absoluteString).inserted }
    }

    init(
        productID: String,
        streamTitleID: String?,
        xboxTitleID: String?,
        name: String,
        publisherName: String,
        description: String,
        tileURL: URL?,
        posterURL: URL?,
        heroURL: URL?,
        artworkURL: URL?,
        categories: [String],
        supportedInputTypes: [String],
        hasEntitlement: Bool?,
        isRecentlyPlayed: Bool?,
        isNew: Bool?,
        lastPlayedAt: Date?,
        playtimeMinutes: Int?,
        achievementProgress: AchievementProgress?,
        recentRank: Int? = nil,
        newRank: Int? = nil, popularRank: Int? = nil, leavingRank: Int? = nil
    ) {
        self.productID = productID
        self.streamTitleID = streamTitleID
        self.xboxTitleID = xboxTitleID
        self.name = name
        self.publisherName = publisherName
        self.description = description
        self.tileURL = tileURL
        self.posterURL = posterURL
        self.heroURL = heroURL
        self.artworkURL = artworkURL
        self.categories = categories
        self.supportedInputTypes = supportedInputTypes
        self.hasEntitlement = hasEntitlement
        self.isRecentlyPlayed = isRecentlyPlayed
        self.isNew = isNew
        self.lastPlayedAt = lastPlayedAt
        self.playtimeMinutes = playtimeMinutes
        self.achievementProgress = achievementProgress
        self.recentRank = recentRank
        self.newRank = newRank
        self.popularRank = popularRank
        self.leavingRank = leavingRank
    }

    init(gameSummary game: GameSummary) {
        productID = game.productID ?? game.id
        streamTitleID = nil
        xboxTitleID = game.titleID
        name = game.name
        publisherName = ""
        description = ""
        tileURL = nil
        posterURL = nil
        heroURL = game.heroURL
        artworkURL = game.artworkURL
        categories = []
        supportedInputTypes = []
        hasEntitlement = nil
        isRecentlyPlayed = nil
        isNew = nil
        lastPlayedAt = game.lastPlayedAt
        playtimeMinutes = game.playtimeMinutes
        achievementProgress = game.achievementProgress
    }
}
