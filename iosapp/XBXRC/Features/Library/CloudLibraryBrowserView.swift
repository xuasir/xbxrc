import SwiftUI

enum CloudLibraryPage: String, AppSecondaryRoute, Sendable {
  case discover, recent, favorites, newGames, popular, leaving, all, search
  var title: String {
    switch self {
    case .discover: "发现"
    case .recent: "最近游玩"
    case .newGames: "新增游戏"
    case .popular: "热门游戏"
    case .leaving: "即将离库"
    case .all: "全部游戏"
    case .favorites: "收藏"
    case .search: "搜索"
    }
  }
  var icon: AppIcon {
    switch self {
    case .discover, .all: .library
    case .recent: .chart
    case .newGames: .cloud
    case .popular: .trophy
    case .leaving: .info
    case .favorites: .star
    case .search: .search
    }
  }
  var subtitle: String {
    switch self {
    case .discover: "云目录栏目与最近游玩"
    case .recent: "继续最近使用的云游戏"
    case .newGames: "最近加入 Game Pass 的游戏"
    case .popular: "Game Pass 热门游戏"
    case .leaving: "即将离开 Game Pass 的游戏"
    case .all: "浏览、分类和控制方式筛选"
    case .favorites: "收藏、置顶与保存的游戏信息"
    case .search: "按游戏名称与发行商查询"
    }
  }
  var systemImage: String {
    switch self {
    case .discover: "sparkles"
    case .recent: "clock.arrow.circlepath"
    case .newGames: "plus.square"
    case .popular: "flame"
    case .leaving: "clock"
    case .all: "square.grid.2x2"
    case .favorites: "star"
    case .search: "magnifyingglass"
    }
  }
  var collection: LibraryCollectionKind? {
    switch self {
    case .recent: .recent
    case .newGames: .newlyAdded
    case .popular: .popular
    case .leaving: .leaving
    case .favorites: .favorites
    default: .all
    }
  }
  var source: String? {
    switch self {
    case .recent: "recent"
    case .newGames: "newest"
    case .popular: "popular"
    case .leaving: "leaving"
    default: nil
    }
  }
  init(collection: LibraryCollectionKind) {
    switch collection {
    case .recent: self = .recent
    case .newlyAdded: self = .newGames
    case .popular: self = .popular
    case .leaving: self = .leaving
    case .favorites: self = .favorites
    case .all: self = .all
    }
  }
}

struct CloudCatalogStatusView: View {
  @ObservedObject var cloud: CloudLibraryStore
  @EnvironmentObject private var auth: AuthStore
  var showsLoading = true
  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      AppAccountRequestNotice()

      if showsLoading || cloud.catalogFailure != nil {
        AppSectionRequestState(
          title: "云游戏目录",
          status: SectionRequestStatus(
            phase: cloud.phase, isBusy: cloud.isRefreshing,
            hasSnapshot: cloud.catalogUpdatedAt != nil,
            updatedAt: cloud.catalogUpdatedAt,
            attempt: cloud.catalogUpdatedAt == nil ? .initial : .refresh,
            failure: cloud.catalogFailure), identifier: "cloud-catalog"
        ) {
          await cloud.refresh(reason: .manualRetry) { try await auth.prepareCloudAccess() }
        }
      }
      if showsLoading || cloud.metadataState.requestStatus.failure != nil {
        AppSectionRequestState(
          title: "游戏图片与资料", status: cloud.metadataState.requestStatus,
          identifier: "cloud-metadata"
        ) {
          await cloud.retryMetadata { try await auth.prepareCloudAccess() }
        }
      }
    }
  }
}

struct CloudSourceRequestStateView: View {
  let source: String
  let title: String
  @EnvironmentObject private var cloud: CloudLibraryStore
  @EnvironmentObject private var auth: AuthStore
  var body: some View {
    AppSectionRequestState(
      title: title,
      status: (cloud.sourceStates[source] ?? XboxSingleDataState()).requestStatus,
      identifier: "cloud-\(source)"
    ) {
      await cloud.retrySource(source) { try await auth.prepareCloudAccess() }
    }
  }
}

struct CloudLibraryBrowserView: View {
  let page: CloudLibraryPage
  var usesSidebar = false
  @EnvironmentObject private var cloud: CloudLibraryStore
  @EnvironmentObject private var favorites: CloudFavoritesStore
  @EnvironmentObject private var auth: AuthStore
  @Environment(\.appPageIsActive) private var isActive
  @State private var search = ""
  @State private var category = "all"
  @State private var touchOnly = false
  @State private var sort: CloudLibrarySort = .source
  @State private var restoredIdentity: String?
  private var baseGames: [CloudLibraryGame] {
    if page == .favorites { return favorites.games(in: cloud.games) }
    return LibraryPresentation.collections(fromCloudGames: cloud.games)
      .first { $0.kind == page.collection }?.games ?? []
  }
  private var categories: [String] { Array(Set(baseGames.flatMap(\.categories))).sorted() }
  private var games: [CloudLibraryGame] {
    let indexed = Array(baseGames.enumerated()).filter { entry in
      let game = entry.element
      return
        (search.isEmpty || game.name.localizedCaseInsensitiveContains(search)
        || game.publisherName.localizedCaseInsensitiveContains(search))
        && (category == "all" || game.categories.contains(category))
        && (!touchOnly
          || game.supportedInputTypes.contains {
            $0.localizedCaseInsensitiveContains("touch")
          })
    }
    return indexed.sorted { lhs, rhs in
      if page == .favorites {
        let leftPinned = favorites.pinnedIDs.contains(lhs.element.productID)
        let rightPinned = favorites.pinnedIDs.contains(rhs.element.productID)
        if leftPinned != rightPinned { return leftPinned }
      }
      if sort == .source { return lhs.offset < rhs.offset }
      if sort == .recent, lhs.element.recentRank != rhs.element.recentRank {
        return (lhs.element.recentRank ?? .max) < (rhs.element.recentRank ?? .max)
      }
      if sort == .newest, lhs.element.newRank != rhs.element.newRank {
        return (lhs.element.newRank ?? .max) < (rhs.element.newRank ?? .max)
      }
      let order = lhs.element.name.localizedStandardCompare(rhs.element.name)
      return order == .orderedSame ? lhs.offset < rhs.offset : order == .orderedAscending
    }.map(\.element)
  }

  var body: some View {
    AppPageScroll {
      VStack(alignment: .leading, spacing: 16) {
        AppContentSearch(text: $search, prompt: "搜索云游戏")
        filters
        Text("\(games.count) 款游戏").appFont(.caption)
          .foregroundStyle(AppThemePalette.textSecondary)
        CloudCatalogStatusView(cloud: cloud)
        if let source = page.source {
          CloudSourceRequestStateView(source: source, title: page.title)
        }
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 148), spacing: 16)], spacing: 16) {
          ForEach(games) { game in
            NavigationLink {
              GameDetailView(game: game)
            } label: {
              gameCard(game)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("cloud-game-\(game.productID)")
            .contextMenu {
              Button(
                favorites.productIDs.contains(game.productID) ? "移除收藏" : "收藏",
                systemImage: "star"
              ) { favorites.toggle(game) }
              Button(
                favorites.pinnedIDs.contains(game.productID) ? "取消置顶" : "置顶收藏",
                systemImage: "pin"
              ) { favorites.togglePin(game) }
              if page == .favorites {
                Button("上移收藏", systemImage: "arrow.up") {
                  favorites.move(game.productID, by: -1)
                }
                Button("下移收藏", systemImage: "arrow.down") {
                  favorites.move(game.productID, by: 1)
                }
              }
            }
          }
        }
        if games.isEmpty, cloud.catalogFailure == nil,
          page.source.flatMap({ cloud.sourceStates[$0]?.failure }) == nil
        {
          AppThemeEmptyState(title: emptyTitle, systemImage: page.systemImage)
            .frame(maxWidth: .infinity)
        }
      }
      .padding(AppLayoutPolicy.pageInset).padding(.bottom, AppLayoutPolicy.pageInset)
      .frame(maxWidth: AppLayoutPolicy.contentMaximumWidth).frame(maxWidth: .infinity)
    }
    .accessibilityIdentifier("cloud-browser-scroll")
    .appThemeCanvas().appPageToolbar(page.title, showsBrand: usesSidebar)
    .task(id: "\(auth.ownerGeneration):\(page.rawValue):\(isActive)") {
      guard isActive else { return }
      let identity = "\(auth.ownerGeneration):\(page.rawValue)"
      guard restoredIdentity != identity else { return }
      restoredIdentity = identity
      if page == .all || page == .search {
        category = favorites.category
        sort = favorites.sort
      }
    }
    .onChange(of: category) { _, _ in persistFilters() }
    .onChange(of: sort) { _, _ in persistFilters() }
    .appRefreshable { await refresh() }
  }

  private var filters: some View {
    AppContentToolbar {
      AppSegmentedFilter(
        selection: $touchOnly, values: [false, true],
        title: { $0 ? "官方触控" : "全部" },
        identifier: "cloud-input-filter", label: "控制方式", fillsWidth: false)
      AppInformationPicker("排序", selection: $sort, selectedTitle: sort.title) {
        ForEach(CloudLibrarySort.allCases) { Text(LocalizedStringKey($0.title)).tag($0) }
      }
      AppInformationPicker(
        "游戏分类", selection: $category, selectedTitle: category == "all" ? "全部分类" : category
      ) {
        Text("全部分类").tag("all")
        ForEach(categories, id: \.self) { Text($0).tag($0) }
      }
      if category != "all" || sort != .source || touchOnly {
        Button("重置筛选") {
          category = "all"
          touchOnly = false
          sort = .source
        }
        .buttonStyle(AppContentCapsuleButtonStyle())
      }
    }
    .accessibilityIdentifier("cloud-browser-tools")
  }
  private func gameCard(_ game: CloudLibraryGame) -> some View {
    VStack(alignment: .leading, spacing: 8) {
      CloudGameRemoteImage(
        candidates: [game.tileURL, game.artworkURL, game.posterURL, game.heroURL].compactMap { $0 },
        contentMode: .fill,
        onSuccess: { cloud.recordSuccessfulImage(productID: game.productID, url: $0) }
      )
      .aspectRatio(1, contentMode: .fit).clipped().clipShape(
        RoundedRectangle(cornerRadius: 8))
      Text(game.name).appFont(.body, weight: .semibold).lineLimit(2)
      if favorites.pinnedIDs.contains(game.productID) {
        Label("已置顶", systemImage: "pin.fill").appFont(.caption).foregroundStyle(
          AppThemePalette.brand)
      } else if favorites.productIDs.contains(game.productID) {
        Label("已收藏", systemImage: "star.fill").appFont(.caption).foregroundStyle(
          AppThemePalette.brand)
      }
      if page == .favorites, !cloud.games.contains(where: { $0.productID == game.productID }) {
        Text("保存的游戏信息").appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
      }
    }.foregroundStyle(.primary).frame(maxWidth: .infinity, alignment: .leading)
  }
  private var emptyTitle: String {
    if cloud.phase == .loading, page != .favorites { return "正在载入云游戏" }
    if let source = page.source, cloud.unavailableSources.contains(source) {
      return "此栏目暂时无法载入"
    }
    if page == .favorites, search.isEmpty, category == "all", !touchOnly { return "暂无收藏" }
    return "当前筛选结果为空"
  }
  private func persistFilters() {
    if page == .all || page == .search {
      favorites.setBrowserPreferences(category: category, sort: sort)
    }
  }
  private func refresh() async {
    if let source = page.source {
      await cloud.retrySource(source) { try await auth.prepareCloudAccess() }
    } else {
      await cloud.refresh(reason: .pullToRefresh) { try await auth.prepareCloudAccess() }
    }
    favorites.updateSnapshots(from: cloud.games)
  }
}
