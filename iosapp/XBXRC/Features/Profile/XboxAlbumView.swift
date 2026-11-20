import AVKit
import SwiftUI

private enum AlbumFilter: String, CaseIterable, Identifiable {
  case all, screenshot, clip
  var id: Self { self }
  var title: String {
    switch self {
    case .all: "全部"
    case .screenshot: "截图"
    case .clip: "游戏片段"
    }
  }
  var kinds: [XboxAlbumKind] {
    switch self {
    case .all: XboxAlbumKind.allCases
    case .screenshot: [.screenshot]
    case .clip: [.clip]
    }
  }
}

struct XboxAlbumView: View {
  var fixedTitleID: String? = nil
  @EnvironmentObject private var auth: AuthStore
  @EnvironmentObject private var data: XboxDataStore
  @Environment(\.appPageIsActive) private var isActive
  @Environment(\.locale) private var locale
  @Environment(\.appInformationIsEmbedded) private var isEmbedded
  @State private var filter: AlbumFilter = .all
  @State private var gameFilter = "all"
  @State private var selectedMedia: XboxAlbumMedia?
  private var sources: [XboxNativeDataState<XboxAlbumMedia>] {
    filter.kinds.map { data.albumStates[$0] ?? XboxNativeDataState() }
  }
  private var items: [XboxAlbumMedia] {
    PersonalLibraryPresentation.album(sources).filter {
      (fixedTitleID == nil || $0.titleID == fixedTitleID)
        && (gameFilter == "all" || $0.titleID == gameFilter)
    }
  }
  private var gameOptions: [(id: String, name: String)] {
    var seen = Set<String>()
    return sources.flatMap(\.items).compactMap { item in
      guard let title = item.titleID, seen.insert(title).inserted else { return nil }
      return (title, item.titleName.isEmpty ? "Xbox 游戏" : item.titleName)
    }.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
  }

  var body: some View {
    Group {
      if isEmbedded {
        albumContent(minimumHeight: 0)
      } else {
        GeometryReader { geometry in
          AppPageScroll {
            albumContent(minimumHeight: geometry.size.height)
          }.accessibilityIdentifier("app-information-scroll")
        }
      }
    }
    .appThemeCanvas()
    .appInformationTypography()
    .task(id: "\(auth.ownerGeneration):\(filter.rawValue):\(isActive)") {
      guard isActive else { return }
      await load()
    }
    .refreshable { await load(refresh: true) }
    .onChange(of: auth.ownerGeneration) { _, _ in
      selectedMedia = nil
      gameFilter = "all"
    }
    .navigationDestination(item: $selectedMedia) { AlbumMediaDetailView(item: $0) }
  }

  private func albumContent(minimumHeight: CGFloat) -> some View {
    VStack(alignment: .leading, spacing: 16) {
      AppAccountRequestNotice()
      if fixedTitleID == nil {
        AppContentToolbar {
          mediaFilter(fillsWidth: false)
          AppInformationPicker(
            "游戏", selection: $gameFilter,
            selectedTitle: gameOptions.first { $0.id == gameFilter }?.name ?? "全部游戏"
          ) {
            Text("全部游戏").tag("all")
            ForEach(gameOptions, id: \.id) { Text($0.name).tag($0.id) }
          }
        }
      } else {
        mediaFilter(fillsWidth: true)
      }
      ForEach(filter.kinds) { kind in
        let state = data.albumStates[kind] ?? XboxNativeDataState()
        if state.requestAttempt != .nextPage {
          AppSectionRequestState(
            title: kind.title, status: state.requestStatus,
            identifier: "album-\(kind.rawValue)"
          ) { await data.loadAlbum(kind: kind, refresh: true) }
        }
      }
      LazyVGrid(columns: [GridItem(.adaptive(minimum: 240), spacing: 16)], spacing: 16) {
        ForEach(items, id: \.queryIdentity) { item in
          Button {
            selectedMedia = item
          } label: {
            mediaCard(item)
          }.buttonStyle(.plain)
            .accessibilityIdentifier("album-media-\(item.queryIdentity)")
        }
      }
      if items.isEmpty, sources.allSatisfy({ $0.phase == .loaded }) {
        Spacer(minLength: 0)
        AppThemeEmptyState(
          title: fixedTitleID == nil && gameFilter == "all" ? "暂无媒体记录" : "当前游戏暂无媒体记录",
          systemImage: "photo.on.rectangle"
        )
        .accessibilityIdentifier("album-empty-state")
        Spacer(minLength: 0)
      }
      ForEach(filter.kinds) { kind in
        let state = data.albumStates[kind] ?? XboxNativeDataState()
        if state.requestAttempt == .nextPage, state.failure != nil {
          AppSectionRequestState(
            title: kind.title, status: state.requestStatus,
            identifier: "album-page-\(kind.rawValue)"
          ) {
            await data.loadAlbum(
              kind: kind, refresh: state.failure?.kind == .repeatedContinuation,
              nextPage: state.failure?.kind != .repeatedContinuation)
          }
        }
      }
      if sources.contains(where: { $0.continuation != nil }) {
        AppLoadMoreButton(isLoading: sources.contains(where: \.isRefreshing)) {
          await load(nextPage: true)
        }
      }
    }
    .padding(AppLayoutPolicy.pageInset).padding(.bottom, AppLayoutPolicy.pageInset)
    .frame(maxWidth: AppLayoutPolicy.contentMaximumWidth, minHeight: minimumHeight)
    .frame(maxWidth: .infinity)
  }

  private func mediaFilter(fillsWidth: Bool) -> some View {
    AppSegmentedFilter(
      selection: $filter, values: AlbumFilter.allCases, title: { $0.title },
      identifier: "album-media-types", label: "媒体类型", fillsWidth: fillsWidth)
  }

  private func mediaCard(_ item: XboxAlbumMedia) -> some View {
    VStack(alignment: .leading, spacing: 8) {
      AsyncImage(url: item.thumbnailURL ?? (item.kind == .screenshot ? item.url : nil)) {
        phase in
        if let image = phase.image {
          image.resizable().scaledToFill()
        } else {
          Rectangle().fill(.quaternary).overlay {
            Image(systemName: item.kind == .clip ? "play.rectangle" : "photo").font(
              .largeTitle
            ).foregroundStyle(AppThemePalette.textSecondary)
          }
        }
      }
      .frame(height: 160).clipped().clipShape(RoundedRectangle(cornerRadius: 8))
      Text(LocalizedStringKey(item.titleName.isEmpty ? item.kind.title : item.titleName))
        .appFont(.body, weight: .semibold).lineLimit(2)
      HStack {
        Text(LocalizedStringKey(item.kind.title))
        if let seconds = item.durationSeconds {
          Text(appLocalizedFormat("%lld 秒", locale: locale, Int64(clamping: seconds)))
            .accessibilityIdentifier("album-duration-\(item.queryIdentity)")
        }
        Spacer(minLength: 0)
      }.appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
      if let date = item.capturedAt {
        Text(
          date.formatted(
            Date.FormatStyle(date: .abbreviated, time: .shortened).locale(locale))
        ).appFont(.caption)
          .accessibilityIdentifier("album-captured-\(item.queryIdentity)")
          .foregroundStyle(AppThemePalette.textSecondary)
      }
      if item.isExpired {
        Text("打开后刷新媒体链接").appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
      }
    }
    .foregroundStyle(.primary).padding(.bottom, 8).frame(
      maxWidth: .infinity, alignment: .leading
    ).contentShape(Rectangle())
  }

  private func load(refresh: Bool = false, nextPage: Bool = false) async {
    // 两来源独立失败和分页，共同投影到相册的时间范围。
    let store = data
    let kinds = filter.kinds
    await withTaskGroup(of: Void.self) { group in
      for kind in kinds {
        group.addTask {
          await store.loadAlbum(kind: kind, refresh: refresh, nextPage: nextPage)
        }
      }
    }
  }
}

private struct AlbumExport: Identifiable {
  let id = UUID()
  let url: URL
}

private struct AlbumMediaDetailView: View {
  @EnvironmentObject private var auth: AuthStore
  @EnvironmentObject private var data: XboxDataStore
  @State private var item: XboxAlbumMedia
  @State private var failure: SectionRequestFailure?
  @State private var failedDownload = false
  @State private var busy = false
  @State private var export: AlbumExport?
  @State private var exportedURL: URL?
  @State private var operation: Task<Void, Never>?
  init(item: XboxAlbumMedia) { _item = State(initialValue: item) }

  var body: some View {
    VStack(spacing: 16) {
      preview.frame(maxWidth: .infinity, maxHeight: .infinity)
      if let failure {
        AppSectionRequestState(
          title: failedDownload ? "媒体下载" : "媒体预览",
          status: SectionRequestStatus(phase: .failed, isBusy: busy, failure: failure),
          identifier: "album-preview"
        ) {
          if failedDownload { await save() } else { await renew() }
        }.padding(.horizontal, AppLayoutPolicy.pageInset)
      }
      if busy { ProgressView("正在处理媒体").padding(.bottom, AppLayoutPolicy.pageInset) }
    }
    .appThemeCanvas().appPageToolbar(item.titleName.isEmpty ? item.kind.title : item.titleName)
    .toolbar {
      ToolbarItem(placement: .topBarTrailing) {
        AppRefreshButton("刷新媒体链接", isRefreshing: busy) { await renew() }
      }.sharedBackgroundVisibility(.hidden)
      ToolbarItem(placement: .topBarTrailing) {
        Menu {
          Button("保存与分享文件", systemImage: "square.and.arrow.up") { run { await save() } }
            .disabled(item.url == nil)
        } label: {
          AppIconView(.share, size: 22).foregroundStyle(AppThemePalette.textPrimary)
        }
        .disabled(busy).accessibilityLabel("媒体操作")
      }.sharedBackgroundVisibility(.hidden)
    }
    .task { if item.isExpired { await renew() } }
    .onDisappear {
      operation?.cancel()
      operation = nil
    }
    .sheet(item: $export, onDismiss: cleanExport) { AlbumFileShareSheet(url: $0.url) }
  }

  @ViewBuilder
  private var preview: some View {
    if item.isExpired {
      AppThemeEmptyState(title: "媒体链接已过期", systemImage: "clock", description: "刷新链接后继续查看。")
    } else if let url = item.url {
      if item.kind == .clip {
        AlbumVideoPlayer(url: url) {
          failure = SectionRequestFailure(kind: .network)
          failedDownload = false
        }
      } else {
        AsyncImage(url: url) { phase in
          switch phase {
          case .success(let image): image.resizable().scaledToFit()
          case .failure(let error):
            AppSectionRequestState(
              title: "截图",
              status: SectionRequestStatus(
                phase: .failed,
                failure: SectionRequestFailure(error: error)),
              identifier: "album-image"
            ) { await renew() }
            .padding(AppLayoutPolicy.pageInset)
          case .empty: ProgressView("正在载入截图")
          @unknown default: ProgressView()
          }
        }
      }
    } else {
      AppThemeEmptyState(title: "媒体链接暂不可用", systemImage: "photo", actionTitle: "刷新链接") {
        run { await renew() }
      }
    }
  }

  private func run(_ action: @escaping @MainActor () async -> Void) {
    guard !busy else { return }
    operation?.cancel()
    operation = Task { await action() }
  }

  private func renew() async {
    guard !busy else { return }
    busy = true
    failure = nil
    failedDownload = false
    let owner = auth.ownerGeneration
    defer { busy = false }
    do {
      let fresh = try await data.renewAlbumMedia(item)
      guard !Task.isCancelled, owner == auth.ownerGeneration else { return }
      item = fresh
      if fresh.isExpired { failure = SectionRequestFailure(kind: .invalidResponse) }
    } catch {
      if !SectionRequestFailure.isCancellation(error) {
        failure = SectionRequestFailure(error: error)
        failedDownload = false
      }
    }
  }

  private func save() async {
    guard !busy else { return }
    if item.isExpired { await renew() }
    guard let url = item.url, !item.isExpired, !Task.isCancelled else { return }
    busy = true
    failure = nil
    let owner = auth.ownerGeneration
    defer { busy = false }
    do {
      let (download, response) = try await URLSession.shared.download(from: url)
      guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode)
      else {
        try? FileManager.default.removeItem(at: download)
        throw XboxNativeQueryError.unavailable
      }
      guard !Task.isCancelled, owner == auth.ownerGeneration else {
        try? FileManager.default.removeItem(at: download)
        return
      }
      cleanExport()
      let suffix = item.kind == .clip ? "mp4" : http.mimeType == "image/png" ? "png" : "jpg"
      let destination = FileManager.default.temporaryDirectory.appendingPathComponent(
        "xbxrc-media-\(UUID().uuidString).\(suffix)")
      try FileManager.default.moveItem(at: download, to: destination)
      exportedURL = destination
      export = AlbumExport(url: destination)
    } catch {
      if !SectionRequestFailure.isCancellation(error), owner == auth.ownerGeneration {
        failure = SectionRequestFailure(error: error)
        failedDownload = true
      }
    }
  }

  private func cleanExport() {
    if let exportedURL { try? FileManager.default.removeItem(at: exportedURL) }
    exportedURL = nil
  }
}

private struct AlbumFileShareSheet: UIViewControllerRepresentable {
  let url: URL
  func makeUIViewController(context: Context) -> UIActivityViewController {
    UIActivityViewController(activityItems: [url], applicationActivities: nil)
  }
  func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
}

private struct AlbumVideoPlayer: View {
  let url: URL
  let onFailure: () -> Void
  @Environment(\.scenePhase) private var scenePhase
  @State private var player: AVPlayer?
  var body: some View {
    VideoPlayer(player: player)
      .task(id: url) {
        let activePlayer = AVPlayer(url: url)
        player = activePlayer
        activePlayer.play()
        while !Task.isCancelled {
          if activePlayer.currentItem?.status == .failed {
            onFailure()
            return
          }
          do { try await Task.sleep(for: .milliseconds(500)) } catch { return }
        }
      }
      .onChange(of: scenePhase) { _, phase in if phase != .active { player?.pause() } }
      .onDisappear {
        player?.pause()
        player = nil
      }
  }
}
