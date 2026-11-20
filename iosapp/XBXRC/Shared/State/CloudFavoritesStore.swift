import Combine
import Foundation

enum CloudLibrarySort: String, CaseIterable, Identifiable, Codable {
    case source, name, recent, newest
    var id: Self { self }
    var title: String {
        switch self {
        case .source: "栏目顺序"
        case .name: "名称"
        case .recent: "最近游玩"
        case .newest: "新增游戏"
        }
    }
}

private struct CloudFavoritePreferences: Codable {
    var version = 2
    var order: [String] = []
    var pinned: Set<String> = []
    var snapshots: [String: CloudLibraryGame] = [:]
    var category = "all"
    var sort: CloudLibrarySort = .name
}

@MainActor
final class CloudFavoritesStore: ObservableObject {
    @Published private(set) var productIDs: Set<String> = []
    @Published private(set) var pinnedIDs: Set<String> = []
    @Published private(set) var order: [String] = []
    @Published private(set) var snapshots: [String: CloudLibraryGame] = [:]
    @Published private(set) var category = "all"
    @Published private(set) var sort: CloudLibrarySort = .name
    private let defaults: UserDefaults
    private var ownerKey: String?
    init(defaults: UserDefaults = .standard) { self.defaults = defaults }

    func bind(accountID: String?) {
        guard accountID != ownerKey else { return }
        ownerKey = accountID
        var preferences = CloudFavoritePreferences()
        if let accountID {
            if let data = defaults.data(forKey: "ios.cloud.preferences.\(accountID)"),
                let saved = try? JSONDecoder().decode(CloudFavoritePreferences.self, from: data),
                saved.version == 2
            {
                preferences = saved
            } else {
                preferences.order =
                    defaults.stringArray(forKey: "ios.cloud.favorites.\(accountID)") ?? []
            }
        }
        var seen = Set<String>()
        order = preferences.order.filter { !$0.isEmpty && seen.insert($0).inserted }
        productIDs = Set(order)
        pinnedIDs = preferences.pinned.intersection(productIDs)
        snapshots = preferences.snapshots.filter {
            productIDs.contains($0.key) && $0.key == $0.value.productID
        }
        category = preferences.category
        sort = preferences.sort
    }

    func toggle(_ productID: String) {
        guard ownerKey != nil, !productID.isEmpty else { return }
        if productIDs.contains(productID) {
            productIDs.remove(productID)
            order.removeAll { $0 == productID }
            pinnedIDs.remove(productID)
            snapshots[productID] = nil
        } else {
            productIDs.insert(productID)
            order.append(productID)
        }
        persist()
    }

    func toggle(_ game: CloudLibraryGame) {
        let adding = !productIDs.contains(game.productID)
        toggle(game.productID)
        if adding, ownerKey != nil {
            snapshots[game.productID] = game
            persist()
        }
    }

    func togglePin(_ game: CloudLibraryGame) {
        guard ownerKey != nil else { return }
        if !productIDs.contains(game.productID) { toggle(game) }
        if pinnedIDs.contains(game.productID) {
            pinnedIDs.remove(game.productID)
        } else {
            pinnedIDs.insert(game.productID)
        }
        persist()
    }

    func move(_ productID: String, by delta: Int) {
        guard let index = order.firstIndex(of: productID), order.indices.contains(index + delta)
        else { return }
        order.swapAt(index, index + delta)
        persist()
    }

    func updateSnapshots(from games: [CloudLibraryGame]) {
        guard ownerKey != nil else { return }
        var changed = false
        for game in games where productIDs.contains(game.productID) {
            if snapshots[game.productID] != game {
                snapshots[game.productID] = game
                changed = true
            }
        }
        if changed { persist() }
    }

    func games(in catalog: [CloudLibraryGame]) -> [CloudLibraryGame] {
        let current = Dictionary(
            catalog.map { ($0.productID, $0) }, uniquingKeysWith: { first, _ in first })
        return order.compactMap { id in
            current[id]
                ?? snapshots[id].map { saved in
                    // 收藏保留最近元数据，游玩前由现有授权与目录查询重新验证身份。
                    CloudLibraryGame(
                        productID: saved.productID, streamTitleID: nil,
                        xboxTitleID: saved.xboxTitleID,
                        name: saved.name, publisherName: saved.publisherName,
                        description: saved.description,
                        tileURL: saved.tileURL, posterURL: saved.posterURL, heroURL: saved.heroURL,
                        artworkURL: saved.artworkURL,
                        categories: saved.categories,
                        supportedInputTypes: saved.supportedInputTypes, hasEntitlement: nil,
                        isRecentlyPlayed: nil, isNew: nil, lastPlayedAt: saved.lastPlayedAt,
                        playtimeMinutes: saved.playtimeMinutes,
                        achievementProgress: saved.achievementProgress)
                }
        }.sorted { pinnedIDs.contains($0.productID) && !pinnedIDs.contains($1.productID) }
    }

    func setBrowserPreferences(category: String, sort: CloudLibrarySort) {
        guard ownerKey != nil else { return }
        self.category = category
        self.sort = sort
        persist()
    }

    private func persist() {
        guard let ownerKey else { return }
        let preferences = CloudFavoritePreferences(
            order: order, pinned: pinnedIDs, snapshots: snapshots, category: category, sort: sort)
        if let data = try? JSONEncoder().encode(preferences) {
            defaults.set(data, forKey: "ios.cloud.preferences.\(ownerKey)")
        }
        defaults.set(order, forKey: "ios.cloud.favorites.\(ownerKey)")
    }
}
