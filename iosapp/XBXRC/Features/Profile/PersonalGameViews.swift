import SwiftUI

enum PersonalGameSection: String, CaseIterable, Identifiable {
  case overview, achievements, album, statistics
  var id: Self { self }
  var title: String {
    switch self {
    case .overview: "概览"
    case .achievements: "成就"
    case .album: "相册"
    case .statistics: "统计"
    }
  }
}

struct GameHistoryView: View {
  var targetID: String? = nil
  var statistics = false
  @EnvironmentObject private var auth: AuthStore
  @EnvironmentObject private var data: XboxDataStore
  @Environment(\.appPageIsActive) private var isActive
  @State private var search = ""
  @State private var filter: PersonalProgressFilter = .all
  @State private var sort: PersonalHistorySort = .recent

  private var state: XboxNativeDataState<GameSummary> { data.historyState(targetID: targetID) }
  private var games: [GameSummary] {
    PersonalLibraryPresentation.history(state.items, search: search, filter: filter, sort: sort)
  }

  var body: some View {
    AppInformationPage {
      Section {
        VStack(alignment: .leading, spacing: 4) {
          if state.hasSnapshot {
            PersonalProgressSummary(games: state.items, statistics: statistics)
          }
          if statistics {
            AppSectionRequestState(
              title: "游玩时长",
              status: (data.historyPlaytimeStates[targetID ?? "self"]
                ?? XboxSingleDataState())
                .requestStatus, identifier: "history-times"
            ) { await data.loadHistoryPlaytimes(targetID: targetID) }
          }
        }
      }
      Section {
        AppContentSearch(text: $search, prompt: "搜索游玩记录")
        AppContentToolbar {
          AppSegmentedFilter(
            selection: $filter, values: PersonalProgressFilter.allCases,
            title: { $0.title }, identifier: "history-progress-filter",
            label: "成就进度", fillsWidth: false)
          AppInformationPicker("排序", selection: $sort, selectedTitle: sort.title) {
            ForEach(PersonalHistorySort.allCases) {
              Text(LocalizedStringKey($0.title)).tag($0)
            }
          }
        }
      }
      if state.requestAttempt != .nextPage {
        AppSectionRequestState(
          title: "游玩记录", status: state.requestStatus, identifier: "history"
        ) {
          await data.loadHistory(targetID: targetID, refresh: true)
        }
      }
      Section {
        AppMediaGrid {
          ForEach(games) { game in
            NavigationLink {
              PersonalGameDetailView(
                game: game, targetID: targetID,
                section: statistics ? .statistics : .achievements)
            } label: {
              PersonalGameCard(game: game, showsPlaytime: statistics)
            }.buttonStyle(.plain)
          }
        }
        if games.isEmpty, state.phase == .loaded {
          Text(
            LocalizedStringKey(search.isEmpty && filter == .all ? "暂无游玩记录" : "当前筛选结果为空")
          )
          .foregroundStyle(AppThemePalette.textSecondary)
        }
      } header: {
        HStack {
          Text("游戏")
          Spacer()
          Text("\(games.count)").appFont(.caption).foregroundStyle(
            AppThemePalette.textSecondary)
        }
      }
      if state.requestAttempt == .nextPage, state.failure != nil {
        AppSectionRequestState(
          title: "更多游玩记录", status: state.requestStatus, identifier: "history-page"
        ) {
          await data.loadHistory(
            targetID: targetID, refresh: state.failure?.kind == .repeatedContinuation,
            nextPage: state.failure?.kind != .repeatedContinuation)
        }
      } else if state.continuation != nil {
        AppLoadMoreButton(isLoading: state.isRefreshing) {
          await data.loadHistory(targetID: targetID, nextPage: true)
          if statistics {
            await data.loadHistoryPlaytimes(targetID: targetID)
          }
        }
      }
    }
    .appThemeCanvas()
    .task(id: "\(auth.ownerGeneration):\(targetID ?? "self"):\(isActive)") {
      guard isActive else { return }
      sort = statistics ? .playtime : .recent
      await load(refresh: false)
    }
    .refreshable { await load(refresh: true) }
  }

  private func load(refresh: Bool) async {
    await data.loadHistory(targetID: targetID, refresh: refresh)
    if statistics {
      await data.loadHistoryPlaytimes(targetID: targetID, refresh: refresh)
    }
  }
}

struct PersonalGameRow: View {
  @Environment(\.locale) private var locale
  let game: GameSummary
  var showsPlaytime = false
  var body: some View {
    HStack(spacing: 12) {
      PersonalArtwork(url: game.artworkURL, size: 52)
      VStack(alignment: .leading, spacing: 5) {
        Text(game.name).appFont(.body, weight: .semibold).foregroundStyle(.primary)
        if showsPlaytime {
          Text(XboxPresentation.playtime(game.playtimeMinutes, locale: locale)).appFont(
            .bodySmall
          )
          .foregroundStyle(AppThemePalette.textSecondary)
        } else if let progress = game.achievementProgress {
          Text(
            "\(progress.unlockedCount)/\(progress.totalCount) 项 · \(progress.earnedGamerscore)/\(progress.totalGamerscore) G"
          )
          .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
          ProgressView(value: Double(progress.percentage), total: 100).tint(
            AppThemePalette.brand)
        } else {
          Text("成就数据暂缺").appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
        }
        if let date = game.lastPlayedAt {
          Text(
            date.formatted(
              Date.FormatStyle(date: .abbreviated, time: .omitted).locale(locale))
          )
          .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
        }
        if PersonalLibraryPresentation.isCompleted(game.achievementProgress) {
          Text("全成就").appFont(.caption, weight: .semibold).foregroundStyle(
            AppThemePalette.brand)
        }
      }
    }
    .padding(.vertical, 4).frame(maxWidth: .infinity, alignment: .leading)
  }
}

struct PersonalArtwork: View {
  let url: URL?
  var size: CGFloat = 64
  var body: some View {
    AsyncImage(url: url) { phase in
      if let image = phase.image {
        image.resizable().scaledToFill()
      } else {
        Rectangle().fill(.quaternary).overlay {
          Image(systemName: "gamecontroller").foregroundStyle(
            AppThemePalette.textSecondary)
        }
      }
    }
    .frame(width: size, height: size).clipShape(RoundedRectangle(cornerRadius: 8))
    .accessibilityHidden(true)
  }
}

/// 根页保留最常用的概况，完整口径和时长明细由统计子页承载。
struct PersonalLibraryOverview: View {
  let games: [GameSummary]
  var body: some View {
    AppMetricGrid {
      AppMetric(title: "游玩过的游戏", value: "\(games.count)")
      AppMetric(
        title: "已解锁成就",
        value:
          games.isEmpty || games.contains(where: { $0.achievementProgress != nil })
          ? "\(games.compactMap(\.achievementProgress).reduce(0) { $0 + $1.unlockedCount })" : "—"
      )
    }
  }
}

struct PersonalProgressSummary: View {
  @Environment(\.locale) private var locale
  let games: [GameSummary]
  var statistics = false
  private var progress: [AchievementProgress] { games.compactMap(\.achievementProgress) }
  var body: some View {
    AppMetricGrid {
      if statistics {
        AppMetric(
          title: "已记录时长",
          value: XboxPresentation.playtime(
            games.isEmpty || games.contains(where: { $0.playtimeMinutes != nil })
              ? games.compactMap(\.playtimeMinutes).reduce(0, +) : nil, locale: locale),
          identifier: "personal-playtime-summary")
      }
      AppMetric(
        title: "全成就游戏",
        value:
          "\(games.filter { PersonalLibraryPresentation.isCompleted($0.achievementProgress) }.count)",
        hint: "当前游戏列表")
      AppMetric(
        title: "成就点数",
        value:
          games.isEmpty || !progress.isEmpty
          ? "\(progress.reduce(0) { $0 + $1.earnedGamerscore }) G" : "—",
        hint: "当前游戏列表")
    }
  }
}

struct PersonalGameCard: View {
  @Environment(\.locale) private var locale
  let game: GameSummary
  var showsPlaytime = false
  var body: some View {
    VStack(alignment: .leading, spacing: AppLayoutPolicy.itemSpacing) {
      Color.clear.aspectRatio(1, contentMode: .fit)
        .overlay {
          AsyncImage(url: game.artworkURL) { image in
            image.resizable().scaledToFill()
          } placeholder: {
            Rectangle().fill(.quaternary).overlay { AppIconView(.controller, size: 40) }
          }
        }
        .clipShape(RoundedRectangle(cornerRadius: 8)).accessibilityHidden(true)
      Text(game.name).appFont(.body, weight: .semibold)
        .foregroundStyle(AppThemePalette.textPrimary)
      if showsPlaytime {
        Text(XboxPresentation.playtime(game.playtimeMinutes, locale: locale)).appFont(
          .caption
        )
        .foregroundStyle(AppThemePalette.textSecondary)
      } else if let progress = game.achievementProgress {
        Text(appLocalizedFormat("%lld%% 成就进度", locale: locale, progress.percentage))
          .appFont(.caption)
          .foregroundStyle(AppThemePalette.textSecondary)
      }
    }
    .frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
    .accessibilityElement(children: .combine)
  }
}

struct PersonalGameDetailView: View {
  @Environment(\.locale) private var locale
  let initialGame: GameSummary
  let targetID: String?
  let installedGame: XboxInstalledGameSummary?
  let host: XboxHostSummary?
  @EnvironmentObject private var auth: AuthStore
  @EnvironmentObject private var data: XboxDataStore
  @EnvironmentObject private var streaming: StreamingFeatureStore
  @EnvironmentObject private var settings: AppSettingsStore
  @Environment(\.appPageIsActive) private var isActive
  @State private var section: PersonalGameSection
  @State private var descriptionExpanded = false

  init(
    game: GameSummary, targetID: String? = nil, section: PersonalGameSection = .overview,
    installedGame: XboxInstalledGameSummary? = nil, host: XboxHostSummary? = nil
  ) {
    initialGame = game
    self.targetID = targetID
    self.installedGame = installedGame
    self.host = host
    _section = State(initialValue: section)
  }

  private var key: String { XboxPersonalQueryKey.game(initialGame.titleID, targetID: targetID) }
  private var detailState: XboxSingleDataState<GameSummary> {
    data.libraryGameStates[key] ?? XboxSingleDataState()
  }
  private var timeState: XboxSingleDataState<Int> {
    data.playerPlaytimeStates[key] ?? XboxSingleDataState()
  }
  private var game: GameSummary {
    detailState.value ?? data.historyState(targetID: targetID).items.first {
      $0.titleID == initialGame.titleID
    } ?? initialGame
  }

  var body: some View {
    VStack(spacing: 0) {
      AppTextTabs(
        selection: $section, values: PersonalGameSection.allCases, title: { $0.title },
        identifier: "personal-game-sections", buttonPrefix: "personal-game-section"
      )
      .frame(maxWidth: AppLayoutPolicy.contentMaximumWidth)
      Group {
        switch section {
        case .overview: overview
        case .achievements: AchievementComparisonView(game: game, targetID: targetID)
        case .album: XboxAlbumView(fixedTitleID: game.titleID)
        case .statistics: statistics
        }
      }
      .frame(maxWidth: .infinity, maxHeight: .infinity)
      .appPageTransition(value: section)
    }
    .appThemeCanvas().appPageToolbar(
      game.name, artwork: .game(game.artworkURL ?? installedGame?.artworkURL)
    )
    .appInformationTypography()
    .toolbar {
      ToolbarItem(placement: .topBarTrailing) {
        AppRefreshButton(
          "刷新统计", isRefreshing: detailState.isRefreshing || timeState.isRefreshing
        ) {
          await refreshCurrentSection()
        }
      }.sharedBackgroundVisibility(.hidden)
    }
    .task(id: "\(auth.ownerGeneration):\(key):\(isActive)") {
      guard isActive else { return }
      await load(refresh: false)
    }
  }

  private var overview: some View {
    AppInformationPage {
      Section {
        HStack(spacing: 16) {
          PersonalArtwork(url: game.artworkURL ?? installedGame?.artworkURL, size: 96)
          VStack(alignment: .leading, spacing: 8) {
            Text(game.name).appFont(.hero, weight: .semibold)
            if let progress = game.achievementProgress {
              Text(
                appLocalizedFormat(
                  "%lld%% 成就进度", locale: locale, progress.percentage)
              ).appFont(.bodySmall)
                .foregroundStyle(AppThemePalette.textSecondary)
            }
            Text(
              XboxPresentation.playtime(
                timeState.hasSnapshot && timeState.failure == nil
                  ? timeState.value : game.playtimeMinutes, locale: locale)
            )
            .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
          }
        }.frame(minHeight: 160, alignment: .bottomLeading).padding(
          .bottom, AppLayoutPolicy.itemSpacing)
        AppSectionRequestState(
          title: "游戏资料", status: detailState.requestStatus, identifier: "personal-game"
        ) {
          await data.loadLibraryGame(
            titleID: initialGame.titleID, targetID: targetID, refresh: true)
        }
        AppSectionRequestState(
          title: "游玩时长", status: timeState.requestStatus, identifier: "personal-game-time"
        ) {
          await data.loadPlayerPlaytime(
            titleID: initialGame.titleID, targetID: targetID, refresh: true)
        }
      }
      if let description = game.description, !description.isEmpty {
        Section {
          Text(description).appFont(.body)
            .lineLimit(descriptionExpanded || description.count <= 140 ? nil : 3)
            .frame(maxWidth: .infinity, alignment: .leading)
        } header: {
          HStack(alignment: .firstTextBaseline) {
            Text("游戏简介")
            Spacer(minLength: 8)
            if description.count > 140 {
              Button(descriptionExpanded ? "收起简介" : "展开简介") {
                descriptionExpanded.toggle()
              }
              .appFont(.caption).foregroundStyle(AppThemePalette.brand)
              .frame(minHeight: 44).buttonStyle(.plain)
              .accessibilityIdentifier("game-description-toggle")
            }
          }
        }
      }
      if let installedGame {
        Section("安装信息") {
          AppMetricGrid {
            if let host { AppMetric(title: "主机", value: host.name) }
            AppMetric(
              title:
                "大小",
              value: installedGame.sizeBytes.map {
                ByteCountFormatter.string(
                  fromByteCount: Int64(clamping: $0), countStyle: .file)
              } ?? "大小未知")
            if let date = installedGame.installedAt {
              AppMetric(
                title:
                  "安装时间",
                value: date.formatted(
                  Date.FormatStyle(date: .abbreviated, time: .omitted).locale(
                    locale)))
            }
            if let date = installedGame.lastPlayedAt {
              AppMetric(
                title:
                  "最近游玩",
                value: date.formatted(
                  Date.FormatStyle(date: .abbreviated, time: .omitted).locale(
                    locale)))
            }
          }
        }
      }
      if let host {
        Section {
          Button("串流此主机", systemImage: "play.fill") {
            if let target = host.streamTargetID {
              streaming.startHome(targetID: target) {
                try await auth.prepareHomeAccess()
              }
            }
          }.disabled(!host.canStartRemotePlay)
        } footer: {
          Text("连接后进入主机界面，选择此游戏开始游玩。")
        }
      }
    }
    .refreshable { await load(refresh: true) }
  }

  private var statistics: some View {
    AppInformationPage {
      Section {
        AppMetricGrid {
          AppMetric(
            title: targetID == nil ? "我的游玩时长" : "玩家游玩时长",
            value: XboxPresentation.playtime(
              timeState.hasSnapshot && timeState.failure == nil
                ? timeState.value : game.playtimeMinutes, locale: locale))
          if let date = game.lastPlayedAt {
            AppMetric(
              title: "最近游玩",
              value: date.formatted(
                Date.FormatStyle(date: .abbreviated, time: .shortened).locale(
                  locale)))
          }
          if let progress = game.achievementProgress {
            AppMetric(
              title: "已解锁成就",
              value: "\(progress.unlockedCount) / \(progress.totalCount)")
            AppMetric(
              title: "成就点数",
              value: "\(progress.earnedGamerscore) / \(progress.totalGamerscore) G")
          }
        }
        if let progress = game.achievementProgress {
          ProgressView(value: Double(progress.percentage), total: 100).tint(
            AppThemePalette.brand)
        }
        AppSectionRequestState(
          title: "游玩时长", status: timeState.requestStatus,
          identifier: "game-statistics-time"
        ) {
          await data.loadPlayerPlaytime(
            titleID: initialGame.titleID, targetID: targetID, refresh: true)
        }
        AppSectionRequestState(
          title: "游戏记录", status: detailState.requestStatus,
          identifier: "game-statistics-record"
        ) {
          await data.loadLibraryGame(
            titleID: initialGame.titleID, targetID: targetID, refresh: true)
        }
      } header: {
        Text("游玩统计")
      } footer: {
        AppInlineHint("时长来自 Xbox，部分游戏受隐私与服务覆盖范围影响。", identifier: "playtime-source")
      }
    }
    .refreshable { await load(refresh: true) }
  }

  private func refreshCurrentSection() async {
    switch section {
    case .overview, .statistics: await load(refresh: true)
    case .achievements:
      async let mine: Void = data.loadPlayerAchievements(
        titleID: game.titleID,
        locale: PreferredGameLocaleNormalizer.normalize(settings.preferredGameLocale),
        refresh: true
      )
      if let targetID {
        await data.loadPlayerAchievements(
          titleID: game.titleID, targetID: targetID,
          locale: PreferredGameLocaleNormalizer.normalize(settings.preferredGameLocale),
          refresh: true)
      }
      await mine
    case .album:
      await withTaskGroup(of: Void.self) { group in
        for kind in XboxAlbumKind.allCases {
          group.addTask { await data.loadAlbum(kind: kind, refresh: true) }
        }
      }
    }
  }

  private func load(refresh: Bool) async {
    async let detail: Void = data.loadLibraryGame(
      titleID: initialGame.titleID, targetID: targetID, refresh: refresh)
    async let time: Void = data.loadPlayerPlaytime(
      titleID: initialGame.titleID, targetID: targetID, refresh: refresh)
    _ = await (detail, time)
  }
}
