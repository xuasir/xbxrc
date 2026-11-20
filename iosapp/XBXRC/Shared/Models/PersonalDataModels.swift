import Foundation

struct XboxFriendSummary: Identifiable, Hashable, Sendable {
    let id: String
    let displayName: String
    let gamertag: String
    let avatarURL: URL?
    let gamerscore: String?
    let presenceState: String?
    let presenceText: String?
    var currentTitleID: String? = nil
}

enum XboxAlbumKind: String, CaseIterable, Identifiable, Sendable {
    case screenshot, clip
    var id: String { rawValue }
    var title: String { self == .screenshot ? "截图" : "游戏片段" }
}

struct XboxAlbumMedia: Identifiable, Hashable, Sendable {
    let id: String
    let kind: XboxAlbumKind
    let titleID: String?
    let titleName: String
    let capturedAt: Date?
    let durationSeconds: UInt64?
    let thumbnailURL: URL?
    let url: URL?
    let expiresAt: Date?
    var pageContinuation: String? = nil
    var queryIdentity: String { "\(kind.rawValue):\(id)" }
    var isExpired: Bool { expiresAt.map { $0 <= Date() } ?? false }
}

struct XboxAlbumPage: Sendable {
    let items: [XboxAlbumMedia]
    let continuation: String?
}

struct XboxInstalledGameSummary: Identifiable, Hashable, Sendable {
    let id: String
    let titleID: String?
    let productID: String?
    let name: String
    let artworkURL: URL?
    let storageDeviceID: String?
    let sizeBytes: UInt64?
    let installedAt: Date?
    let lastPlayedAt: Date?
}

struct XboxInstalledLibrarySnapshot: Sendable {
    let consoleID: String
    var games: [XboxInstalledGameSummary]?
    var storage: [XboxHostStorageSummary]?
    var powerState: String?
    var notices: [String]
}

struct XboxNativeDataState<Item: Sendable>: Sendable {
    var items: [Item] = []
    var phase: DataLoadPhase = .idle
    var isRefreshing = false
    var errorMessage: String?
    var continuation: String?
    var seenContinuations: Set<String> = []
    var updatedAt: Date?
    var failure: SectionRequestFailure?
    var requestAttempt: SectionRequestAttempt = .initial
}

struct XboxInstalledLibraryState: Sendable {
    var snapshot: XboxInstalledLibrarySnapshot?
    var phase: DataLoadPhase = .idle
    var isRefreshing = false
    var errorMessage: String?
    var scopeID: String?
    var updatedAt: Date?
    var sections: [XboxInstalledSource: XboxSingleDataState<Bool>] = [:]
}

struct XboxDataRequestContext: Sendable {
    let token: String
    let ownerGeneration: UInt64?
    let fallbackOwnerKey: String?
}

struct XboxHistoryPage: Sendable {
    let items: [GameSummary]
    let continuation: String?
}

struct XboxAchievementsPage: Sendable {
    let items: [AchievementSummary]
    let continuation: String?
}

struct XboxTitlePlaytimeSnapshot: Sendable {
    let items: [TitlePlaytime]
    var failedTitleIDs: Set<String> = []
    var failures: [String: SectionRequestFailure] = [:]
}

struct XboxSingleDataState<Item: Sendable>: Sendable {
    var value: Item?
    var phase: DataLoadPhase = .idle
    var isRefreshing = false
    var errorMessage: String?
    var updatedAt: Date?
    var failure: SectionRequestFailure?
    var requestAttempt: SectionRequestAttempt = .initial
}

struct InstalledSectionResult: Sendable {
    var snapshot: XboxInstalledLibrarySnapshot
    var artwork: [CloudCatalogMetadata] = []
    var failure: SectionRequestFailure?
}

enum XboxPersonalQueryKey {
    static func game(_ titleID: String, targetID: String?) -> String {
        "\(targetID ?? "self"):\(titleID)"
    }
    static func achievements(_ titleID: String, targetID: String?, locale: String) -> String {
        "\(game(titleID, targetID: targetID)):\(locale)"
    }
}

enum PersonalProgressFilter: String, CaseIterable, Identifiable {
    case all, inProgress, completed
    var id: Self { self }
    var title: String {
        switch self {
        case .all: "全部"
        case .inProgress: "进行中"
        case .completed: "已完成"
        }
    }
}

enum PersonalHistorySort: String, CaseIterable, Identifiable {
    case recent, playtime, name
    var id: Self { self }
    var title: String {
        switch self {
        case .recent: "最近游玩"
        case .playtime: "游玩时长"
        case .name: "名称"
        }
    }
}

enum InstalledGameSort: String, CaseIterable, Identifiable {
    case recent, name, installed, size
    var id: Self { self }
    var title: String {
        switch self {
        case .recent: "最近游玩"
        case .name: "名称"
        case .installed: "安装时间"
        case .size: "占用空间"
        }
    }
}

struct AchievementComparisonRow: Identifiable, Hashable {
    let own: AchievementSummary?
    let friend: AchievementSummary?
    var item: AchievementSummary { own ?? friend! }
    var id: String { item.queryIdentity }
}

enum PersonalLibraryPresentation {
    static func isCompleted(_ progress: AchievementProgress?) -> Bool {
        guard let progress else { return false }
        if progress.totalCount > 0 { return progress.unlockedCount >= progress.totalCount }
        return progress.totalGamerscore > 0 && progress.earnedGamerscore >= progress.totalGamerscore
    }

    static func history(
        _ games: [GameSummary], search: String, filter: PersonalProgressFilter,
        sort: PersonalHistorySort
    ) -> [GameSummary] {
        games.filter { game in
            guard search.isEmpty || game.name.localizedCaseInsensitiveContains(search) else {
                return false
            }
            switch filter {
            case .all: return true
            case .inProgress:
                return !isCompleted(game.achievementProgress)
                    && (game.achievementProgress?.unlockedCount ?? 0) > 0
            case .completed: return isCompleted(game.achievementProgress)
            }
        }.sorted { lhs, rhs in
            if sort == .playtime, lhs.playtimeMinutes != rhs.playtimeMinutes {
                return (lhs.playtimeMinutes ?? -1) > (rhs.playtimeMinutes ?? -1)
            }
            if sort == .recent, lhs.lastPlayedAt != rhs.lastPlayedAt {
                return (lhs.lastPlayedAt ?? .distantPast) > (rhs.lastPlayedAt ?? .distantPast)
            }
            let comparison = lhs.name.localizedStandardCompare(rhs.name)
            return comparison == .orderedSame
                ? lhs.titleID < rhs.titleID : comparison == .orderedAscending
        }
    }

    static func installed(
        _ games: [XboxInstalledGameSummary], search: String, storageID: String,
        sort: InstalledGameSort
    ) -> [XboxInstalledGameSummary] {
        games.filter {
            (search.isEmpty || $0.name.localizedCaseInsensitiveContains(search))
                && (storageID == "all" || $0.storageDeviceID == storageID)
        }.sorted { lhs, rhs in
            if sort == .size, lhs.sizeBytes != rhs.sizeBytes {
                return (lhs.sizeBytes ?? 0) > (rhs.sizeBytes ?? 0)
            }
            let leftDate = sort == .installed ? lhs.installedAt : lhs.lastPlayedAt
            let rightDate = sort == .installed ? rhs.installedAt : rhs.lastPlayedAt
            if sort == .recent || sort == .installed, leftDate != rightDate {
                return (leftDate ?? .distantPast) > (rightDate ?? .distantPast)
            }
            let comparison = lhs.name.localizedStandardCompare(rhs.name)
            return comparison == .orderedSame ? lhs.id < rhs.id : comparison == .orderedAscending
        }
    }

    /// 总览仅汇总完整容量事实，异常可用值按该设备总容量收敛。
    static func storageOverview(_ devices: [XboxHostStorageSummary]) -> XboxHostStorageSummary? {
        guard !devices.isEmpty else { return nil }
        var free: UInt64 = 0
        var total: UInt64 = 0
        for device in devices {
            guard let deviceFree = device.freeBytes, let deviceTotal = device.totalBytes,
                deviceTotal > 0
            else { return nil }
            let nextFree = free.addingReportingOverflow(min(deviceFree, deviceTotal))
            let nextTotal = total.addingReportingOverflow(deviceTotal)
            guard !nextFree.overflow, !nextTotal.overflow else { return nil }
            free = nextFree.partialValue
            total = nextTotal.partialValue
        }
        return XboxHostStorageSummary(id: "all", name: "全部设备", freeBytes: free, totalBytes: total)
    }

    static func achievementRows(own: [AchievementSummary], friend: [AchievementSummary])
        -> [AchievementComparisonRow]
    {
        let ownByID = Dictionary(
            own.map { ($0.queryIdentity, $0) }, uniquingKeysWith: { first, _ in first })
        let friendByID = Dictionary(
            friend.map { ($0.queryIdentity, $0) }, uniquingKeysWith: { first, _ in first })
        var seen = Set<String>()
        return (own + friend).filter { seen.insert($0.queryIdentity).inserted }.map {
            AchievementComparisonRow(
                own: ownByID[$0.queryIdentity], friend: friendByID[$0.queryIdentity])
        }
    }

    // 两种媒体共用已覆盖的时间区间，继续分页后再展开更早的记录。
    static func album(_ states: [XboxNativeDataState<XboxAlbumMedia>]) -> [XboxAlbumMedia] {

        let watermark = states.compactMap { state -> Date? in
            guard state.continuation != nil, state.errorMessage == nil, state.phase == .loaded
            else {
                return nil
            }
            return state.items.compactMap(\.capturedAt).min()
        }.max()
        return states.flatMap(\.items).filter { item in
            guard let watermark else { return true }
            return item.capturedAt.map { $0 >= watermark } ?? false
        }.sorted {
            if $0.capturedAt != $1.capturedAt {
                return ($0.capturedAt ?? .distantPast) > ($1.capturedAt ?? .distantPast)
            }
            return $0.queryIdentity < $1.queryIdentity
        }
    }
}
