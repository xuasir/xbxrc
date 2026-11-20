import Foundation

enum LibraryCollectionKind: String, CaseIterable, Identifiable, Sendable {
    case recent
    case favorites
    case newlyAdded
    case popular
    case leaving
    case all

    var id: Self { self }
    var requestSource: String? {
        switch self {
        case .recent: "recent"
        case .newlyAdded: "newest"
        case .popular: "popular"
        case .leaving: "leaving"
        default: nil
        }
    }

    var title: String {
        switch self {
        case .recent:
            "最近游玩"
        case .newlyAdded:
            "新入库"
        case .favorites: "收藏"
        case .popular: "热门游戏"
        case .leaving: "即将离库"
        case .all:
            "全部云游戏"
        }
    }

}

struct LibraryCollection: Identifiable, Equatable, Sendable {
    let kind: LibraryCollectionKind
    let games: [CloudLibraryGame]

    var id: LibraryCollectionKind { kind }
    var title: String { kind.title }
    var homeGames: [CloudLibraryGame] {
        Array(games.prefix(LibraryLayoutMetrics.homeShelfLimit))
    }
}

enum LibraryPresentation {
    static func collections(from games: [GameSummary]) -> [LibraryCollection] {
        collections(fromCloudGames: games.map(CloudLibraryGame.init(gameSummary:)))
    }

    static func collections(
        fromCloudGames games: [CloudLibraryGame], favorites: [CloudLibraryGame] = [],
        visibleSources: Set<String> = []
    ) -> [LibraryCollection] {
        guard !games.isEmpty || !favorites.isEmpty || !visibleSources.isEmpty else {
            return []
        }

        return LibraryCollectionKind.allCases.compactMap { kind in
            let sortedGames = kind == .favorites ? favorites : sorted(games, for: kind)
            guard
                !sortedGames.isEmpty || kind == .all
                    || kind.requestSource.map({ visibleSources.contains($0) }) == true
            else {
                return nil
            }
            return LibraryCollection(kind: kind, games: sortedGames)
        }
    }

    static func heroGames(from games: [GameSummary]) -> [CloudLibraryGame] {
        heroGames(fromCloudGames: games.map(CloudLibraryGame.init(gameSummary:)))
    }

    static func heroGames(fromCloudGames games: [CloudLibraryGame]) -> [CloudLibraryGame] {
        let recentGames = sorted(games, for: .recent)
        let source = recentGames.isEmpty ? sorted(games, for: .all) : recentGames
        return Array(source.prefix(LibraryLayoutMetrics.heroLimit))
    }

    static func metadata(for game: GameSummary, kind: LibraryCollectionKind) -> String {
        metadata(for: CloudLibraryGame(gameSummary: game), kind: kind)
    }

    static func metadata(for game: CloudLibraryGame, kind: LibraryCollectionKind) -> String {
        switch kind {
        case .recent:
            guard let lastPlayedAt = game.lastPlayedAt else {
                return "最近游玩时间未知"
            }
            let relativeDate = lastPlayedAt.formatted(
                .relative(presentation: .named, unitsStyle: .wide)
            )
            return "上次游玩 · \(relativeDate)"
        case .newlyAdded:
            if let category = game.categories.first {
                return "Game Pass 新入库 · \(category)"
            }
            return "Game Pass 新入库"
        case .favorites: return "已收藏"
        case .popular: return "Game Pass 热门游戏"
        case .leaving: return "即将离开 Game Pass"
        case .all:
            if let minutes = game.playtimeMinutes {
                return playtime(minutes)
            }
            if let lastPlayedAt = game.lastPlayedAt {
                let relativeDate = lastPlayedAt.formatted(
                    .relative(presentation: .named, unitsStyle: .wide)
                )
                return "上次游玩 · \(relativeDate)"
            }
            return "Xbox 游戏"
        }
    }

    private static func sorted(
        _ games: [CloudLibraryGame],
        for kind: LibraryCollectionKind
    ) -> [CloudLibraryGame] {
        let indexedGames = Array(games.enumerated())

        switch kind {
        case .recent:
            if indexedGames.allSatisfy({ $0.element.isRecentlyPlayed != nil }) {
                return
                    indexedGames
                    .filter { $0.element.isRecentlyPlayed == true }
                    .sorted {
                        ($0.element.recentRank ?? $0.offset) < ($1.element.recentRank ?? $1.offset)
                    }
                    .map(\.element)
            }
            return indexedGames.compactMap {
                entry -> (offset: Int, element: CloudLibraryGame)? in
                // xCloud MRU 是最近游玩栏目的事实来源；旧活动模型缺少该标记时回退日期。
                guard
                    entry.element.isRecentlyPlayed == true
                        || (entry.element.isRecentlyPlayed == nil
                            && entry.element.lastPlayedAt != nil)
                else {
                    return nil
                }
                return entry
            }.sorted { lhs, rhs in
                if lhs.element.isRecentlyPlayed != rhs.element.isRecentlyPlayed {
                    return lhs.element.isRecentlyPlayed == true
                }
                switch (lhs.element.lastPlayedAt, rhs.element.lastPlayedAt) {
                case (let left?, let right?):
                    if left != right {
                        return left > right
                    }
                    return nameThenIndex(lhs, rhs)
                case (_?, nil):
                    return true
                case (nil, _?):
                    return false
                case (nil, nil):
                    // 日期缺失时保留服务端顺序。
                    return lhs.offset < rhs.offset
                }
            }.map(\.element)
        case .newlyAdded:
            return
                indexedGames
                .filter { $0.element.isNew == true }
                .sorted { ($0.element.newRank ?? $0.offset) < ($1.element.newRank ?? $1.offset) }
                .map(\.element)
        case .popular:
            return indexedGames.filter { $0.element.popularRank != nil }
                .sorted {
                    ($0.element.popularRank ?? $0.offset) < ($1.element.popularRank ?? $1.offset)
                }
                .map(\.element)
        case .leaving:
            return indexedGames.filter { $0.element.leavingRank != nil }
                .sorted {
                    ($0.element.leavingRank ?? $0.offset) < ($1.element.leavingRank ?? $1.offset)
                }
                .map(\.element)
        case .favorites: return games
        case .all:
            return indexedGames.sorted(by: nameThenIndex).map(\.element)
        }
    }

    private static func nameThenIndex(
        _ lhs: (offset: Int, element: CloudLibraryGame),
        _ rhs: (offset: Int, element: CloudLibraryGame)
    ) -> Bool {
        let comparison = lhs.element.name.localizedStandardCompare(rhs.element.name)
        if comparison != .orderedSame {
            return comparison == .orderedAscending
        }
        return lhs.offset < rhs.offset
    }

    private static func playtime(_ minutes: Int?) -> String {
        guard let minutes else {
            return "时长未知"
        }
        if minutes < 60 {
            return "\(minutes) 分钟"
        }
        let hours = Double(minutes) / 60
        if hours < 10 {
            return "\(hours.formatted(.number.precision(.fractionLength(1)))) 小时"
        }
        return "\(Int(hours.rounded())) 小时"
    }
}
