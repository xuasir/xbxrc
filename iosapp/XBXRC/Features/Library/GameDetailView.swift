import SwiftUI

struct GameDetailView: View {
  @EnvironmentObject private var authStore: AuthStore
  @EnvironmentObject private var dataStore: XboxDataStore
  @EnvironmentObject private var cloudStore: CloudLibraryStore
  @EnvironmentObject private var streamingStore: StreamingFeatureStore
  @EnvironmentObject private var settings: AppSettingsStore
  @EnvironmentObject private var favorites: CloudFavoritesStore
  @EnvironmentObject private var touchLayouts: SavedTouchLayoutsStore
  @Environment(\.appPageIsActive) private var isActive
  @Environment(\.locale) private var locale
  private let initialGame: CloudLibraryGame
  var onPlay: ((String) -> Void)?
  @State private var selectedScreenshot: CloudScreenshotItem?
  @State private var descriptionExpanded = false

  init(game: CloudLibraryGame, onPlay: ((String) -> Void)? = nil) {
    initialGame = game
    self.onPlay = onPlay
  }
  init(gameSummary: GameSummary, onPlay: ((String) -> Void)? = nil) {
    initialGame = CloudLibraryGame(gameSummary: gameSummary)
    self.onPlay = onPlay
  }
  private var game: CloudLibraryGame {
    cloudStore.games.first { $0.productID == initialGame.productID } ?? initialGame
  }
  private var detailState: XboxSingleDataState<CloudProductDetails> {
    cloudStore.productDetailState(game.productID)
  }
  private var detail: CloudProductDetails? { detailState.value }
  private var artworkCandidates: [URL] {
    var seen = Set<URL>()
    return [
      detail?.heroURL, game.heroURL, initialGame.heroURL,
      detail?.posterURL, game.posterURL, initialGame.posterURL,
      detail?.tileURL, game.tileURL, game.artworkURL,
      initialGame.tileURL, initialGame.artworkURL,
    ].compactMap { $0 }.filter { seen.insert($0).inserted }
  }
  private var coverCandidates: [URL] {
    [detail?.tileURL, game.tileURL, game.artworkURL, detail?.posterURL, game.posterURL]
      .compactMap { $0 }
  }
  private var waitState: XboxSingleDataState<UInt64> {
    cloudStore.waitTimeState(productID: game.productID)
  }
  private var canPlay: Bool { authStore.isSignedIn && game.hasEntitlement != false }
  @Environment(\.appFloatingNavigationInset) private var navigationInset
  @State private var playBarHeight: CGFloat = 64
  private var personalGame: GameSummary? {
    guard let titleID = game.xboxTitleID else { return nil }
    return GameSummary(
      id: titleID, titleID: titleID, name: detail?.name ?? game.name,
      artworkURL: detail?.tileURL ?? game.tileURL ?? game.artworkURL,
      heroURL: detail?.heroURL ?? game.heroURL ?? initialGame.heroURL,
      lastPlayedAt: personalLastPlayed,
      playtimeMinutes: personalPlaytime, achievementProgress: personalProgress,
      description: detail?.description ?? game.description, productID: game.productID)
  }

  var body: some View {
    AppPageScroll(bottomClearance: playBarHeight + 16) {
      VStack(alignment: .leading, spacing: 32) {
        AppAccountRequestNotice()
        header
        touchLayoutsSection
        personalSection
        productSection
        screenshots
        languages
        accessibility
      }
      .padding(AppLayoutPolicy.pageInset).padding(.bottom, AppLayoutPolicy.pageInset)
      .frame(maxWidth: AppLayoutPolicy.contentMaximumWidth, alignment: .leading).frame(
        maxWidth: .infinity)
    }
    .accessibilityIdentifier("cloud-detail-scroll")
    .appThemeCanvas().appPageToolbar(
      detail?.name ?? game.name,
      artwork: .cloudGame(artworkCandidates),
      onArtworkSuccess: {
        cloudStore.recordSuccessfulImage(productID: game.productID, url: $0)
      }
    )
    .appInformationTypography()
    .overlay(alignment: .bottom) {
      playBar
        .onGeometryChange(for: CGFloat.self) {
          $0.size.height
        } action: {
          playBarHeight = $0
        }
        .padding(.bottom, navigationInset)
    }
    .toolbar {
      ToolbarItem(placement: .topBarTrailing) {
        Button {
          favorites.toggle(game)
        } label: {
          AppIconView(
            .star, size: 22, selected: favorites.productIDs.contains(game.productID)
          )
          .foregroundStyle(AppThemePalette.textPrimary)
        }.accessibilityLabel(
          favorites.productIDs.contains(game.productID) ? "移除收藏" : "收藏游戏")
      }.sharedBackgroundVisibility(.hidden)
      ToolbarItem(placement: .topBarTrailing) {
        Menu {
          Button(
            favorites.pinnedIDs.contains(game.productID) ? "取消置顶" : "置顶收藏",
            systemImage: "pin"
          ) { favorites.togglePin(game) }
          Button("刷新游戏资料", systemImage: "arrow.clockwise") {
            Task { await load(refresh: true) }
          }
        } label: {
          AppIconView(.sliders, size: 22).foregroundStyle(AppThemePalette.textPrimary)
        }
        .accessibilityLabel("游戏操作")
      }.sharedBackgroundVisibility(.hidden)
    }
    .task(
      id:
        "\(authStore.ownerGeneration):\(cloudStore.productScopeID):\(game.productID):\(settings.mobile.cloudServerRegion):\(isActive)"
    ) {
      guard isActive, authStore.isSignedIn else { return }
      await load(refresh: false)
    }
    .appRefreshable { await load(refresh: true) }
    .sheet(item: $selectedScreenshot) { screenshot in
      NavigationStack { CloudScreenshotPreview(url: screenshot.url) }
        .presentationDragIndicator(.visible)
    }
    .onChange(of: authStore.ownerGeneration) { _, _ in selectedScreenshot = nil }
  }

  private var header: some View {
    HStack(alignment: .bottom, spacing: 16) {
      CloudGameRemoteImage(
        candidates: coverCandidates, contentMode: .fill,
        onSuccess: { cloudStore.recordSuccessfulImage(productID: game.productID, url: $0) }
      )
      .frame(width: 80, height: 80).clipped()
      .clipShape(RoundedRectangle(cornerRadius: 8))
      .accessibilityHidden(true)
      .accessibilityIdentifier("cloud-detail-cover")
      VStack(alignment: .leading, spacing: 8) {
        Text(detail?.name ?? game.name).appFont(.hero, weight: .semibold)
        Text(detail?.publisherName ?? game.publisherName).appFont(.caption).foregroundStyle(
          AppThemePalette.textSecondary)
        if favorites.pinnedIDs.contains(game.productID) {
          Label("已置顶", systemImage: "pin.fill").appFont(.caption).foregroundStyle(
            AppThemePalette.brand)
        }
        AppWrappingLayout(spacing: 12, lineSpacing: 4) {
          Label("云游戏", systemImage: "cloud")
          if game.supportedInputTypes.contains(where: {
            $0.localizedCaseInsensitiveContains("touch")
          }) {
            Label("官方触控", systemImage: "hand.tap")
          }
          if game.supportedInputTypes.contains(where: {
            ["controller", "gamepad"].contains($0.lowercased())
          }) {
            Label("手柄", systemImage: "gamecontroller")
          }
        }.appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
      }
    }
    .frame(
      maxWidth: .infinity, minHeight: artworkCandidates.isEmpty ? 0 : 216,
      alignment: .bottomLeading
    )
  }

  private var touchLayoutsSection: some View {
    NavigationLink {
      OfficialTouchLayoutsView(game: game)
    } label: {
      HStack(spacing: 12) {
        AppIconView(.touch).foregroundStyle(AppThemePalette.brand)
        VStack(alignment: .leading, spacing: 4) {
          Text("查看游戏布局").appFont(.body, weight: .semibold)
          Text("保存游戏适配布局，用于 Home 串流")
            .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
          if touchLayouts.layouts.contains(where: { $0.productId == game.productID }) {
            Text("已保存到本机").appFont(.caption).foregroundStyle(AppThemePalette.brand)
          }
        }
        Spacer(minLength: 8)
        AppIconView(.chevron, size: 16).foregroundStyle(AppThemePalette.brand)
      }
      .padding(16).frame(minHeight: 44).contentShape(Rectangle())
      .appSurface(radius: AppLayoutPolicy.cardRadius)
    }
    .buttonStyle(.plain).accessibilityIdentifier("cloud-official-touch-layouts")
  }

  private var recordState: XboxSingleDataState<GameSummary> {
    guard let title = game.xboxTitleID else { return XboxSingleDataState() }
    return dataStore.libraryGameStates[XboxPersonalQueryKey.game(title, targetID: nil)]
      ?? XboxSingleDataState()
  }
  private var recordTimeState: XboxSingleDataState<Int> {
    guard let title = game.xboxTitleID else { return XboxSingleDataState() }
    return dataStore.playerPlaytimeStates[XboxPersonalQueryKey.game(title, targetID: nil)]
      ?? XboxSingleDataState()
  }
  private var personalPlaytime: Int? {
    recordTimeState.hasSnapshot ? recordTimeState.value : game.playtimeMinutes
  }
  private var personalProgress: AchievementProgress? {
    recordState.hasSnapshot ? recordState.value?.achievementProgress : game.achievementProgress
  }
  private var personalLastPlayed: Date? {
    recordState.hasSnapshot ? recordState.value?.lastPlayedAt : game.lastPlayedAt
  }
  private var personalSection: some View {
    VStack(alignment: .leading, spacing: 16) {
      Text("我的游戏记录").appFont(.section, weight: .semibold)
      AppSectionRequestState(
        title: "我的游戏记录", status: recordState.requestStatus, identifier: "cloud-record"
      ) {
        if let title = game.xboxTitleID {
          await dataStore.loadLibraryGame(titleID: title, refresh: true)
        }
      }
      AppSectionRequestState(
        title: "游玩时长", status: recordTimeState.requestStatus,
        identifier: "cloud-record-time"
      ) {
        if let title = game.xboxTitleID {
          await dataStore.loadPlayerPlaytime(titleID: title, refresh: true)
        }
      }
      if recordState.hasSnapshot, recordState.value == nil, recordState.failure == nil {
        Text("暂无此游戏的游玩记录").appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
      }
      if personalPlaytime != nil || personalProgress != nil || personalLastPlayed != nil {
        HStack {
          if personalPlaytime != nil {
            Text(XboxPresentation.playtime(personalPlaytime, locale: locale))
              .appFont(.body)
          }
          Spacer(minLength: 8)
          if let progress = personalProgress {
            Text(appLocalizedFormat("%lld%% 成就进度", locale: locale, progress.percentage))
              .appFont(.bodySmall).foregroundStyle(
                AppThemePalette.textSecondary)
          }
        }
        if let date = personalLastPlayed {
          Text(
            "上次游玩 · \(date.formatted(Date.FormatStyle(date: .abbreviated, time: .omitted).locale(locale)))"
          )
          .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
        }
      }
      if let personalGame {
        NavigationLink {
          PersonalGameDetailView(game: personalGame, section: .achievements)
        } label: {
          HStack(spacing: 12) {
            AppIconView(.trophy).foregroundStyle(AppThemePalette.brand)
            VStack(alignment: .leading, spacing: 4) {
              Text("成就、相册与统计").appFont(.body, weight: .semibold)
              Text("查看我的游戏资料与解锁进度").appFont(.caption)
                .foregroundStyle(AppThemePalette.textSecondary)
            }
            Spacer(minLength: 8)
            AppIconView(.chevron, size: 16).foregroundStyle(AppThemePalette.brand)
          }.padding(12).frame(minHeight: 44).contentShape(Rectangle())
            .appSurface(radius: AppLayoutPolicy.cardRadius)
        }.buttonStyle(.plain)
      }
    }
  }

  private var productSection: some View {
    VStack(alignment: .leading, spacing: 24) {
      let description = detail?.description ?? game.description
      VStack(alignment: .leading, spacing: 8) {
        HStack(alignment: .firstTextBaseline) {
          Text("关于游戏").appFont(.section, weight: .semibold)

          Spacer(minLength: 8)
          if description.count > 140 {
            descriptionToggle
          }
        }
        AppSectionRequestState(
          title: "游戏资料", status: detailState.requestStatus,
          identifier: "cloud-product"
        ) {
          await cloudStore.loadProductDetail(productID: game.productID, refresh: true)
        }
        if !description.isEmpty {
          Text(description).appFont(.body).fixedSize(horizontal: false, vertical: true)
            .lineLimit(descriptionExpanded || description.count <= 140 ? nil : 3)
        }
      }
      VStack(alignment: .leading, spacing: 12) {
        Text("发行信息").appFont(.section, weight: .semibold)
        productFact("开发商", detail?.developer)
        productFact("发行商", detail?.publisherName ?? game.publisherName)
        productFact("发行日期", releaseDateText)
        productFact("分级", detail?.rating)
        productFact("分类", (detail?.categories ?? game.categories).joined(separator: " · "))
        productFact("平台", detail?.platforms.joined(separator: " · "))
      }
      .padding(16).frame(maxWidth: .infinity, alignment: .leading)
      .appSurface(radius: AppLayoutPolicy.cardRadius)
      .accessibilityIdentifier("cloud-release-card")
      if let attributes = detail?.attributes, !attributes.isEmpty {
        VStack(alignment: .leading, spacing: 12) {
          Text("游戏特性").appFont(.section, weight: .semibold)
          AppWrappingLayout(spacing: 8, lineSpacing: 8) {
            ForEach(Array(attributes.enumerated()), id: \.offset) { _, attribute in
              Text(attribute).appFont(.caption)
                .padding(.horizontal, 10).padding(.vertical, 6)
                .background(
                  AppThemePalette.brand.opacity(0.08),
                  in: RoundedRectangle(cornerRadius: 6))
            }
          }
        }.padding(16).frame(maxWidth: .infinity, alignment: .leading)
          .appSurface(radius: AppLayoutPolicy.cardRadius)
          .accessibilityIdentifier("cloud-features-card")
      }
    }
  }

  private var descriptionToggle: some View {
    Button {
      descriptionExpanded.toggle()
    } label: {
      HStack(spacing: 4) {
        Text(descriptionExpanded ? "收起简介" : "展开简介")
        AppIconView(.chevron, size: 10)
          .rotationEffect(.degrees(descriptionExpanded ? -90 : 90))
      }.appFont(.caption).foregroundStyle(AppThemePalette.brand).frame(minHeight: 44)
    }
    .buttonStyle(.plain).accessibilityIdentifier("cloud-description-toggle")
  }

  private var releaseDateText: String? {
    guard let value = detail?.releaseDate, !value.isEmpty else { return nil }
    let formatter = DateFormatter()
    formatter.locale = Locale(identifier: "en_US_POSIX")
    formatter.calendar = Calendar(identifier: .gregorian)
    formatter.timeZone = TimeZone(secondsFromGMT: 0)
    formatter.dateFormat = "yyyy-MM-dd"
    guard let date = formatter.date(from: String(value.prefix(10))) else { return value }
    return date.formatted(Date.FormatStyle(date: .abbreviated, time: .omitted).locale(locale))
  }

  @ViewBuilder
  private func productFact(_ title: String, _ value: String?) -> some View {
    if let value, !value.isEmpty {
      HStack(alignment: .firstTextBaseline, spacing: 12) {
        Text(LocalizedStringKey(title)).appFont(.caption)
          .foregroundStyle(AppThemePalette.textSecondary)
          .frame(minWidth: 56, alignment: .leading)
        Text(value).appFont(.body)
      }
      .fixedSize(horizontal: false, vertical: true)
      .accessibilityElement(children: .combine)
      .accessibilityIdentifier("cloud-product-fact-\(title)")
    }
  }

  @ViewBuilder
  private var screenshots: some View {
    if let screenshots = detail?.screenshots, !screenshots.isEmpty {
      VStack(alignment: .leading, spacing: 16) {
        Text("游戏截图").appFont(.section, weight: .semibold)
        ScrollView(.horizontal) {
          LazyHStack(spacing: 16) {
            ForEach(Array(screenshots.enumerated()), id: \.offset) { index, url in
              Button {
                selectedScreenshot = CloudScreenshotItem(url: url)
              } label: {
                CloudGameRemoteImage(
                  candidates: [url], contentMode: .fill, onSuccess: { _ in }
                )
                .frame(width: 280, height: 158).clipped().clipShape(
                  RoundedRectangle(cornerRadius: 8)
                )
                .contentShape(Rectangle())
              }.buttonStyle(.plain)
                .accessibilityLabel("查看游戏截图 \(index + 1)")
                .accessibilityIdentifier("cloud-screenshot-\(index)")
            }
          }
        }
        .frame(height: 158)
        .scrollIndicators(.hidden)
      }
    }
  }

  @ViewBuilder
  private var languages: some View {
    if let languages = detail?.languages, !languages.isEmpty {
      VStack(alignment: .leading, spacing: 16) {
        Text("语言支持").appFont(.section, weight: .semibold)
        ForEach(languages.sorted { $0.locale < $1.locale }) { language in
          VStack(alignment: .leading, spacing: 6) {
            Text(
              locale.localizedString(forIdentifier: language.locale)
                ?? language.locale
            )
            .appFont(.body, weight: .semibold)
            AppWrappingLayout(spacing: 16, lineSpacing: 4) {
              productFact(
                "界面",
                appLocalizedString(supportText(language.interface), locale: locale))
              productFact(
                "配音",
                appLocalizedString(supportText(language.audio), locale: locale))
              productFact(
                "字幕",
                appLocalizedString(supportText(language.subtitles), locale: locale))
            }
          }.frame(maxWidth: .infinity, alignment: .leading)
        }
      }
    }
  }

  @ViewBuilder
  private var accessibility: some View {
    if let features = detail?.accessibility, !features.isEmpty {
      VStack(alignment: .leading, spacing: 16) {
        Text("无障碍支持").appFont(.section, weight: .semibold)
        VStack(alignment: .leading, spacing: 12) {
          ForEach(Array(features.enumerated()), id: \.offset) { _, feature in
            Text(feature).appFont(.body)
          }
        }.frame(maxWidth: .infinity, alignment: .leading)
      }
    }
  }

  private var playBar: some View {
    ViewThatFits(in: .horizontal) {
      HStack(alignment: .center, spacing: 16) {
        playStatus
        Spacer(minLength: 8)
        playAction
      }
      VStack(alignment: .trailing, spacing: 8) {
        playStatus
          .frame(maxWidth: .infinity, alignment: .leading)
        playAction
      }
    }
    .padding(12)
    .frame(maxWidth: .infinity, alignment: .leading)
    .appSurface(radius: AppLayoutPolicy.cardRadius, prominent: true)
    .padding(.horizontal, AppLayoutPolicy.pageInset).padding(.vertical, 8)
    .frame(maxWidth: AppLayoutPolicy.contentMaximumWidth, alignment: .leading)
    .frame(maxWidth: .infinity, alignment: .center)
  }

  private var playAction: some View {
    Button(action: handlePlay) {
      Label("游玩", systemImage: "play.fill")
    }
    .buttonStyle(AppCompactPlayButtonStyle()).disabled(!canPlay)
    .opacity(canPlay ? 1 : 0.45)
    .accessibilityLabel("游玩 \(game.name)")
    .accessibilityIdentifier("cloud-play-action")
  }

  private var playStatus: some View {
    VStack(alignment: .leading, spacing: 4) {
      if game.hasEntitlement == false {
        Text("当前账户需要对应的云游戏访问权限")
      } else if let seconds = waitState.value {
        Text(
          seconds == 0
            ? appLocalizedString("当前预计可直接进入", locale: locale)
            : appLocalizedFormat("预计等待 %@", locale: locale, waitText(seconds)))
      } else {
        Text("准备好就开始游玩")
      }
    }
    .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
    .fixedSize(horizontal: false, vertical: true)
  }

  private func supportText(_ value: UInt64?) -> String {
    switch value {
    case 1: "支持"
    case 0: "暂无支持"
    case .none: "信息暂缺"
    case .some: "信息暂缺"
    }
  }
  private func waitText(_ seconds: UInt64) -> String {
    seconds < 60
      ? appLocalizedFormat("%@ 秒", locale: locale, seconds.formatted(.number.locale(locale)))
      : appLocalizedFormat(
        "%@ 分钟", locale: locale,
        Int((Double(seconds) / 60).rounded(.up)).formatted(.number.locale(locale)))
  }
  private func handlePlay() {
    guard canPlay else { return }
    if let onPlay, let streamTitleID = game.streamTitleID {
      onPlay(streamTitleID)
    } else {
      streamingStore.startCloudGame(productID: game.productID) { startup in
        try await authStore.prepareCloudAccess(startup: startup)
      }
    }
  }
  private func load(refresh: Bool) async {
    async let record: Void = loadRecord(refresh: refresh)
    async let product: Void = cloudStore.loadProductDetail(
      productID: game.productID, refresh: refresh)
    if let streamTitleID = game.streamTitleID {
      await cloudStore.loadWaitTime(
        productID: game.productID, streamTitleID: streamTitleID, refresh: refresh
      ) {
        try await authStore.prepareCloudAccess()
      }
    }
    _ = await (product, record)
  }
  private func loadRecord(refresh: Bool) async {
    if let title = game.xboxTitleID {
      async let record: Void = dataStore.loadLibraryGame(titleID: title, refresh: refresh)
      async let time: Void = dataStore.loadPlayerPlaytime(titleID: title, refresh: refresh)
      _ = await (record, time)
    }
  }
}

private struct CloudScreenshotItem: Identifiable {
  let url: URL
  var id: String { url.absoluteString }
}

private struct CloudScreenshotPreview: View {
  let url: URL
  @State private var revision = 0
  var body: some View {
    AsyncImage(url: url) { phase in
      switch phase {
      case .success(let image): image.resizable().scaledToFit()
      case .failure(let error):
        AppSectionRequestState(
          title: "截图",
          status: SectionRequestStatus(
            phase: .failed,
            failure: SectionRequestFailure(error: error)),
          identifier: "cloud-screenshot"
        ) { revision += 1 }
        .padding(AppLayoutPolicy.pageInset)
      case .empty: ProgressView("正在载入截图")
      @unknown default: ProgressView()
      }
    }
    .id(revision).frame(maxWidth: .infinity, maxHeight: .infinity)
    .appThemeCanvas().appPageToolbar("游戏截图", backIdentifier: "cloud-screenshot-close")
    .toolbar {
      ToolbarItem(placement: .topBarTrailing) {
        ShareLink(item: url) {
          AppIconView(.share, size: 22).foregroundStyle(AppThemePalette.textPrimary)
        }
        .accessibilityLabel("分享截图")
      }.sharedBackgroundVisibility(.hidden)
    }
  }
}
