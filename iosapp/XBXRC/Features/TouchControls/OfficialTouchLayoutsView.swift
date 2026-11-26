import SwiftUI

struct GameTouchLayoutsLink: View {
  let game: CloudLibraryGame
  let identifier: String
  @EnvironmentObject private var library: SavedTouchLayoutsStore

  var body: some View {
    NavigationLink {
      OfficialTouchLayoutsView(game: game)
    } label: {
      HStack(spacing: 12) {
        AppIconView(.touch).foregroundStyle(AppThemePalette.brand)
        VStack(alignment: .leading, spacing: 4) {
          Text("查看游戏布局").appFont(.body, weight: .semibold)
          Text("保存游戏适配布局，用于 Home 串流")
            .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
          if library.layouts.contains(where: { $0.productId == game.productID }) {
            Text("已保存到本机").appFont(.caption).foregroundStyle(AppThemePalette.brand)
          }
        }
        Spacer(minLength: 8)
        AppIconView(.chevron, size: 16).foregroundStyle(AppThemePalette.brand)
      }
      .padding(16).frame(minHeight: 44).contentShape(Rectangle())
      .appSurface(radius: AppLayoutPolicy.cardRadius)
    }
    .buttonStyle(.plain).accessibilityIdentifier(identifier)
  }
}

struct OfficialTouchLayoutsView: View {
  let game: CloudLibraryGame
  @EnvironmentObject private var auth: AuthStore
  @EnvironmentObject private var cloud: CloudLibraryStore
  @EnvironmentObject private var library: SavedTouchLayoutsStore
  @EnvironmentObject private var settings: AppSettingsStore
  @State private var catalog: OfficialTouchCatalog?
  @State private var querying = false
  @State private var queryError: SectionRequestFailure?
  @State private var queryRevision = 0
  @State private var queryRequestRevision: UInt64 = 0
  private var queryIdentity: String {
    "\(auth.ownerGeneration):\(cloud.productScopeID):\(game.productID):\(queryRevision)"
  }
  var body: some View {
    AppPageScroll {
      LazyVStack(alignment: .leading, spacing: 16) {
        AppAccountRequestNotice()
        VStack(alignment: .leading, spacing: 8) {
          Text(verbatim: game.name).appFont(.section, weight: .semibold)
          Text("保存后可在 Home 串流菜单中切换布局。")
            .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
        }.padding(.bottom, 8)
        if !auth.isSignedIn {
          Text("登录后查看游戏触屏布局").appFont(.body)
        } else if querying {
          ProgressView("正在查询游戏布局").frame(minHeight: 80)
        } else if let queryError {
          GameTouchRequestFailure(
            error: queryError, title: "游戏触屏布局", identifier: "official-touch-query"
          ) { queryRevision += 1 }
        } else if let catalog {
          ForEach(catalog.sources.filter { $0.failure != nil }, id: \.source.rawValue) {
            source in
            if let failure = source.failure {
              GameTouchRequestFailure(
                error: failure, title: source.source.title,
                identifier: "game-touch-source-\(source.source.rawValue)"
              ) { queryRevision += 1 }
            }
          }
          if catalog.bundles.isEmpty && catalog.sources.allSatisfy({ $0.failure == nil }) {
            VStack(alignment: .leading, spacing: 8) {
              Text("此游戏暂无可用的手柄布局").appFont(.body)
              if game.supportedInputTypes.contains(where: {
                $0.lowercased().contains("nativetouch")
              }) {
                Text("游戏提供原生触屏操作，本机布局库使用手柄映射布局。")
                  .appFont(.caption).foregroundStyle(
                    AppThemePalette.textSecondary)
              }
              Button("重新查询") { queryRevision += 1 }.frame(minHeight: 44)
            }.accessibilityIdentifier("official-touch-empty")
          }
          ForEach(catalog.bundles) { descriptor in
            GameTouchBundleCards(
              game: game, titleID: catalog.titleID, descriptor: descriptor
            )
            .id("\(queryIdentity):\(descriptor.id)")
          }
        }
      }.padding(AppLayoutPolicy.pageInset)
        .frame(maxWidth: AppLayoutPolicy.contentMaximumWidth, alignment: .leading).frame(
          maxWidth: .infinity)
    }.appThemeCanvas().appPageToolbar("游戏触屏布局").appInformationTypography()
      .task(id: queryIdentity) { await queryCatalog() }
      .appRefreshable { await queryCatalog(refresh: true) }
      .toolbar {
        ToolbarItem(placement: .topBarTrailing) {
          AppRefreshButton("重新查询", isRefreshing: querying) {
            await queryCatalog(refresh: true)
          }
          .disabled(!auth.isSignedIn)
          .accessibilityIdentifier("game-touch-catalog-refresh")
        }.sharedBackgroundVisibility(.hidden)
      }
  }
  private func queryCatalog(refresh: Bool = false) async {
    queryRequestRevision &+= 1
    let revision = queryRequestRevision
    let request = queryIdentity
    catalog = nil
    querying = false
    queryError = nil
    guard auth.isSignedIn else { return }
    querying = true
    defer { if request == queryIdentity, revision == queryRequestRevision { querying = false } }
    let client = library.client
    let game = game
    let region = settings.mobile.cloudServerRegion
    do {
      let result = try await client.catalog(
        game: game, serverRegion: region, refresh: refresh || queryRevision > 0,
        prepareAccess: {
          try await cloud.withCloudAccess(
            prepareAccess: { try await auth.prepareCloudAccess() },
            operation: { $0 })
        })
      try Task.checkCancellation()
      guard request == queryIdentity, revision == queryRequestRevision else { return }
      catalog = result
    } catch {
      guard request == queryIdentity, revision == queryRequestRevision, !Task.isCancelled
      else { return }
      queryError = SectionRequestFailure(error: error)
    }
  }

}

private struct GameTouchBundleCards: View {
  let game: CloudLibraryGame
  let titleID: String
  let descriptor: XboxOfficialTouchBundleChoice
  @EnvironmentObject private var auth: AuthStore
  @EnvironmentObject private var cloud: CloudLibraryStore
  @EnvironmentObject private var library: SavedTouchLayoutsStore
  @Environment(\.locale) private var locale
  @Environment(\.displayScale) private var displayScale
  @State private var bundle: (any OfficialTouchBundleProviding)?
  @State private var bundleOwner: UInt64?
  @State private var downloading = false
  @State private var error: SectionRequestFailure?
  @State private var revision = 0
  private var requestIdentity: String {
    "\(auth.ownerGeneration):\(cloud.productScopeID):\(descriptor.id):\(locale.identifier):\(displayScale):\(revision)"
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      if downloading {
        HStack(spacing: 12) {
          ProgressView()
          VStack(alignment: .leading, spacing: 6) {
            Text("正在下载布局包").appFont(.body)
            GameTouchSourceVersion(
              descriptor: descriptor, version: descriptor.tabVersion)
          }
        }.padding(16).frame(maxWidth: .infinity, alignment: .leading).appSurface(
          radius: AppLayoutPolicy.cardRadius)
      } else if let error {
        GameTouchRequestFailure(
          error: error, title: descriptor.source.title,
          identifier: "game-touch-download-\(descriptor.id)"
        ) { revision += 1 }
      } else if let bundle, let owner = bundleOwner {
        if bundle.choices.isEmpty { Text("布局包中暂无可用布局").appFont(.body) }
        ForEach(bundle.choices) { choice in
          GameTouchLayoutCard(
            context: .init(
              game: game, titleID: titleID, descriptor: descriptor, bundle: bundle,
              choice: choice, owner: owner))
        }
      }
    }.task(id: requestIdentity) { await download() }
  }
  private func download() async {
    let request = requestIdentity
    let owner = auth.ownerGeneration
    bundle = nil
    bundleOwner = nil
    error = nil
    guard auth.isSignedIn else { return }
    downloading = true
    defer { if request == requestIdentity { downloading = false } }
    let client = library.client
    let game = game
    let titleID = titleID
    let bundleID = descriptor.id
    let language = locale.identifier.replacingOccurrences(of: "_", with: "-")
    let scale = Double(displayScale)
    do {
      let result = try await client.download(
        titleID: titleID, bundleID: bundleID, game: game,
        locale: language, displayScale: scale,
        prepareAccess: {
          try await cloud.withCloudAccess(
            prepareAccess: { try await auth.prepareCloudAccess() },
            operation: { $0 })
        })
      try Task.checkCancellation()
      guard request == requestIdentity, owner == auth.ownerGeneration else { return }
      bundle = result
      bundleOwner = owner
    } catch {
      guard request == requestIdentity, !Task.isCancelled
      else { return }
      self.error = SectionRequestFailure(error: error)
    }
  }
}

private struct GameTouchLayoutContext: Identifiable {
  let game: CloudLibraryGame
  let titleID: String
  let descriptor: XboxOfficialTouchBundleChoice
  let bundle: any OfficialTouchBundleProviding
  let choice: XboxTouchLayoutChoice
  let owner: UInt64
  var id: String { descriptor.id + ":" + choice.id }
  func saved(in records: [XboxSavedTouchLayout]) -> XboxSavedTouchLayout? {
    records.first {
      $0.xboxTitleId == titleID && $0.bundleId == descriptor.savedBundleID
        && $0.layoutId == choice.id
    }
  }
}

private struct GameTouchLayoutCard: View {
  let context: GameTouchLayoutContext
  @EnvironmentObject private var auth: AuthStore
  @EnvironmentObject private var library: SavedTouchLayoutsStore
  @State private var saving = false
  @State private var capabilityIssues: [String] = []
  @State private var errorMessage: String?
  @State private var saveTask: Task<Void, Never>?
  @State private var previewContext: GameTouchLayoutContext?
  @State private var saveButtonWidth: CGFloat = 110
  private var saved: XboxSavedTouchLayout? { context.saved(in: library.layouts) }
  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      ZStack(alignment: .trailing) {
        Button {
          previewContext = context
        } label: {
          VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
              Text(verbatim: context.choice.name).appFont(.body, weight: .semibold)
                .lineLimit(1)
              if context.choice.isDefault {
                Text("默认布局").appFont(.caption).foregroundStyle(
                  AppThemePalette.brand
                )
                .fixedSize()
              }
              Spacer(minLength: 8)

            }
            VStack(alignment: .leading, spacing: 4) {
              GameTouchSourceVersion(
                descriptor: context.descriptor, version: context.bundle.version)
              Text(
                saved == nil
                  ? "尚未保存"
                  : saved?.bundleVersion == context.bundle.version
                    ? "已保存到本机" : "有新版本可更新"
              )
              .appFont(.caption)
              .foregroundStyle(
                saved == nil ? AppThemePalette.textSecondary : AppThemePalette.brand
              )
              .accessibilityIdentifier("game-touch-saved-\(context.id)")
            }
            .lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
          }
          .padding(.trailing, saveButtonWidth + 12)
          .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
          .contentShape(Rectangle())
        }
        .buttonStyle(.plain).disabled(saving)
        .accessibilityIdentifier("game-touch-preview-\(context.id)")
        Button(action: saveLayout) {
          if saving {
            ProgressView()
          } else {
            Text(
              saved == nil
                ? "保存到本机"
                : saved?.bundleVersion == context.bundle.version ? "重新保存" : "更新布局")
          }
        }
        .buttonStyle(AppContentCapsuleButtonStyle()).frame(minHeight: 44)
        .fixedSize(horizontal: true, vertical: false)
        .onGeometryChange(for: CGFloat.self) {
          $0.size.width
        } action: {
          saveButtonWidth = $0
        }
        .disabled(
          saving || library.isSaving || !capabilityIssues.isEmpty
        )
        .accessibilityIdentifier("game-touch-save-\(context.id)")
      }
      if !capabilityIssues.isEmpty || !context.choice.issues.isEmpty {
        if !capabilityIssues.isEmpty {
          Text("布局等待适配").appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
        }
        ForEach(Array(Set(capabilityIssues + context.choice.issues)).sorted(), id: \.self) {
          issue in
          Text(verbatim: issue).appFont(.caption).foregroundStyle(
            AppThemePalette.textSecondary)
        }
      }
      if let errorMessage {
        Text(verbatim: errorMessage).appFont(.caption).foregroundStyle(
          AppThemePalette.textSecondary)
      }
    }.padding(12).frame(maxWidth: .infinity, alignment: .leading).appSurface(
      radius: AppLayoutPolicy.cardRadius
    )
    .accessibilityElement(children: .contain).accessibilityIdentifier(
      "game-touch-card-\(context.id)"
    )
    .fullScreenCover(
      item: $previewContext, onDismiss: { AppOrientationCoordinator.shared.refresh() }
    ) { selection in
      OfficialTouchLayoutPreviewView(
        bundle: selection.bundle, choice: selection.choice,
        gameName: selection.game.name, ownerGeneration: selection.owner
      )
      .environment(\.appFloatingNavigationInset, 0).environment(\.appCanvasBounds, nil)
    }
    .onDisappear { saveTask?.cancel() }.onChange(of: auth.ownerGeneration) { _, _ in
      saveTask?.cancel()
    }
  }
  private func saveLayout() {
    guard context.owner == auth.ownerGeneration else { return }
    saveTask?.cancel()
    saveTask = Task { @MainActor in
      saving = true
      errorMessage = nil
      capabilityIssues = []
      defer { saving = false }
      do {
        let snapshot = try await context.bundle.preview(layoutID: context.choice.id)
        let prepared = try await PreparedTouchLayout.prepare(snapshot)
        try Task.checkCancellation()
        guard context.owner == auth.ownerGeneration else { return }
        guard prepared.layout.report.isRunnable else {
          capabilityIssues = prepared.layout.report.unsupportedMessages
          return
        }
        try await library.save(bundle: context.bundle, prepared: prepared)
      } catch {
        guard !SectionRequestFailure.isCancellation(error), !Task.isCancelled else {
          return
        }
        errorMessage = SectionRequestFailure(error: error).message
      }
    }
  }
}

private struct GameTouchSourceVersion: View {
  let descriptor: XboxOfficialTouchBundleChoice
  let version: String
  @Environment(\.locale) private var locale
  var body: some View {
    Text(
      verbatim: appLocalizedString(descriptor.source.shortTitle, locale: locale) + " · "
        + appLocalizedFormat("版本 %@", locale: locale, version)
    )
    .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
  }
}

struct GameTouchRequestFailure: View {
  let error: SectionRequestFailure
  let title: String
  let identifier: String
  let retry: @MainActor () -> Void
  var body: some View {
    AppSectionRequestState(
      title: title, status: SectionRequestStatus(phase: .failed, failure: error),
      identifier: identifier, showsAuthenticationFailure: true
    ) { retry() }
  }
}
