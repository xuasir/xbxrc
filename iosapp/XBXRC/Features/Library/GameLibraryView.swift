import SwiftUI

struct GameLibraryView: View {
  @EnvironmentObject private var authStore: AuthStore
  @EnvironmentObject private var cloudStore: CloudLibraryStore
  @EnvironmentObject private var settingsStore: AppSettingsStore
  @EnvironmentObject private var favorites: CloudFavoritesStore
  @Environment(\.dynamicTypeSize) private var dynamicTypeSize
  @State private var page: CloudLibraryPage = .discover
  @State private var didTraceAppearance = false
  let isActive: Bool

  init(isActive: Bool = true) {
    self.isActive = isActive
  }

  private var activationID: String {
    "\(authStore.ownerGeneration):\(isActive):\(settingsStore.mobile.catalogMarket):\(settingsStore.mobile.catalogLanguage)"
  }

  private var collections: [LibraryCollection] {
    LibraryPresentation.collections(
      fromCloudGames: cloudStore.games, favorites: favorites.games(in: cloudStore.games),
      visibleSources: Set(cloudStore.sourceStates.keys))
  }

  var body: some View {
    GeometryReader { geometry in
      let usesSidebar =
        geometry.size.width >= AppLayoutPolicy.secondarySidebarThreshold
        && !dynamicTypeSize.isAccessibilitySize
      NavigationStack(
        path: Binding(
          get: { page == .discover ? [] : [page] },
          set: { page = $0.last ?? .discover }
        )
      ) {
        AppSecondaryPageLayout(usesSidebar: usesSidebar, sidebar: { cloudSidebar }) {
          content
        }
        .appThemeCanvas()
        .appPageToolbar("云游戏", showsBrand: true)
        .toolbar {
          ToolbarItem(placement: .topBarTrailing) {
            NavigationLink(value: CloudLibraryPage.favorites) {
              AppIconView(.star, size: 22)
                .foregroundStyle(AppThemePalette.textPrimary)
            }
            .accessibilityLabel("收藏")
            .accessibilityIdentifier("cloud-open-favorites")
          }
          .sharedBackgroundVisibility(.hidden)
          ToolbarItem(placement: .topBarTrailing) {
            NavigationLink(value: CloudLibraryPage.search) {
              AppIconView(.search, size: 22)
                .foregroundStyle(AppThemePalette.textPrimary)
            }
            .accessibilityLabel("搜索云游戏")
          }
          .sharedBackgroundVisibility(.hidden)
        }
        .navigationDestination(for: CloudLibraryPage.self) { route in
          AppSecondaryPageLayout(
            usesSidebar: usesSidebar, sidebar: { cloudSidebar }
          ) {
            CloudLibraryBrowserView(page: route, usesSidebar: usesSidebar)
          }
        }
      }
      .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
    .onChange(of: authStore.ownerGeneration) { _, _ in page = .discover }
    .onChange(of: cloudStore.games, initial: true) { _, games in
      favorites.updateSnapshots(from: games)
    }
    .task(id: activationID) {
      guard isActive, authStore.phase == .signedIn else { return }
      await activateCloudLibrary()
    }
    .onAppear {
      guard !didTraceAppearance else {
        return
      }
      didTraceAppearance = true
      IOSRuntimeTrace.event(
        domain: "library-ui",
        event: "libraryPageAppeared",
        payload: [
          "signedIn": .bool(authStore.isSignedIn),
          "games": .integer(Int64(cloudStore.games.count)),
        ],
        dimension: .frontend,
        importance: .key
      )
    }
  }

  @ViewBuilder
  private var content: some View {
    if !authStore.isSignedIn {
      XboxLoginView(
        isBusy: authStore.isBusy,
        errorMessage: authStore.errorMessage
      ) {
        Task {
          await authStore.retry()
        }
      }
      .frame(maxWidth: .infinity, maxHeight: .infinity)
    } else if cloudStore.games.isEmpty && favorites.games(in: cloudStore.games).isEmpty {
      emptyLibraryContent
    } else {
      libraryContent
    }
  }

  private var libraryContent: some View {
    GeometryReader { geometry in
      AppPageScroll {
        LazyVStack(spacing: 0) {
          HStack {
            Text("\(cloudStore.games.count) 款游戏")
              .appFont(.bodySmall).foregroundStyle(AppThemePalette.textSecondary)
            Spacer()
          }
          .padding(.horizontal, AppLayoutPolicy.pageInset)
          .padding(.vertical, 24)

          CloudCatalogStatusView(cloud: cloudStore)
            .padding(.horizontal, AppLayoutPolicy.pageInset).padding(
              .bottom, AppLayoutPolicy.pageInset)

          ForEach(collections) { collection in
            LibraryShelf(collection: collection)
              .padding(.bottom, 32)
          }
        }
        .padding(.bottom, 24)
        .frame(maxWidth: AppLayoutPolicy.contentMaximumWidth).frame(maxWidth: .infinity)
      }
      .background(.clear)
      .accessibilityIdentifier("cloud-library-scroll")
      .refreshable {
        await refreshCloudLibrary(reason: .pullToRefresh)
      }
    }
    .onAppear {
      IOSRuntimeTrace.state(
        domain: "library-ui",
        event: "contentPresented",
        payload: [
          "games": .integer(Int64(cloudStore.games.count)),
          "collections": .integer(Int64(collections.count)),
        ],
        dimension: .frontend,
        importance: .key
      )
    }
  }

  @ViewBuilder
  private var emptyLibraryContent: some View {
    switch cloudStore.phase {
    case .idle, .loading:
      GameLibraryLoadingView()
        .refreshable {
          await refreshCloudLibrary(reason: .pullToRefresh)
        }
    case .failed:
      refreshableEmptyState {
        CloudCatalogStatusView(cloud: cloudStore)
          .padding(AppLayoutPolicy.pageInset)
      }
    case .loaded:
      refreshableEmptyState {
        AppThemeEmptyState(
          title: "暂无游戏记录",
          systemImage: "play.rectangle.on.rectangle"
        )
      }
    }
  }

  private func refreshableEmptyState<Content: View>(
    @ViewBuilder content: @escaping () -> Content
  ) -> some View {
    GeometryReader { geometry in
      AppPageScroll {
        content()
          .frame(maxWidth: .infinity, minHeight: geometry.size.height)
      }
      .refreshable {
        await refreshCloudLibrary(reason: .pullToRefresh)
      }
    }
  }

  private var cloudSidebar: some View {
    AppPageScroll {
      VStack(alignment: .leading, spacing: 12) {
        Text("云游戏").appFont(.section, weight: .semibold).frame(minHeight: 44)
        VStack(spacing: 4) {
          ForEach(CloudLibraryPage.allCases) { route in
            Button {
              page = route
            } label: {
              AppSidebarNavigationRow(title: route.title, selected: page == route)
            }
            .buttonStyle(.plain).accessibilityAddTraits(
              page == route ? .isSelected : []
            )
            .accessibilityIdentifier("section-\(route.rawValue)")
          }
        }
      }
      .padding(.horizontal, 16).padding(.top, 24)
      .padding(.bottom, AppLayoutPolicy.pageInset)
    }
    .appInformationTypography()
    .accessibilityIdentifier("secondary-sidebar")
    .overlay(alignment: .trailing) { Rectangle().fill(AppThemePalette.border).frame(width: 1) }
  }

  private func activateCloudLibrary() async {
    let operationID = UUID().uuidString
    IOSRuntimeTrace.event(
      domain: "library-ui",
      event: "libraryActivationRequested",
      payload: ["signedIn": .bool(authStore.isSignedIn)],
      dimension: .frontend,
      importance: .key,
      operationID: operationID
    )
    await cloudStore.activate(session: authStore.session) {
      try await authStore.prepareCloudAccess()
    }
    IOSRuntimeTrace.snapshot(
      domain: "library-ui",
      event: "libraryActivationCompleted",
      payload: [
        "phase": .string(String(describing: cloudStore.phase)),
        "cacheState": .string(cloudStore.cacheState.rawValue),
        "games": .integer(Int64(cloudStore.games.count)),
      ],
      dimension: .frontend,
      importance: .key,
      operationID: operationID
    )
  }

  private func refreshCloudLibrary(reason: CloudCatalogRefreshReason) async {
    let operationID = UUID().uuidString
    IOSRuntimeTrace.event(
      domain: "library-ui",
      event: "userRefreshRequested",
      payload: [
        "reason": .string(reason.rawValue),
        "existingGames": .integer(Int64(cloudStore.games.count)),
      ],
      dimension: .frontend,
      importance: .key,
      operationID: operationID
    )
    await cloudStore.refresh(reason: reason) {
      try await authStore.prepareCloudAccess()
    }
    IOSRuntimeTrace.snapshot(
      domain: "library-ui",
      event: "userRefreshCompleted",
      payload: [
        "reason": .string(reason.rawValue),
        "phase": .string(String(describing: cloudStore.phase)),
        "games": .integer(Int64(cloudStore.games.count)),
      ],
      dimension: .frontend,
      importance: .key,
      operationID: operationID
    )
  }

}

struct AppThemeEmptyState: View {
  let title: String
  let systemImage: String
  let description: String?
  let actionTitle: String?
  let action: (() -> Void)?

  init(
    title: String,
    systemImage: String,
    description: String? = nil,
    actionTitle: String? = nil,
    action: (() -> Void)? = nil
  ) {
    self.title = title
    self.systemImage = systemImage
    self.description = description
    self.actionTitle = actionTitle
    self.action = action
  }

  var body: some View {
    VStack(spacing: 20) {
      Image(systemName: systemImage)
        .font(.system(size: 38, weight: .medium))
        .symbolRenderingMode(.hierarchical)
        .foregroundStyle(AppThemePalette.textSecondary)
        .accessibilityHidden(true)

      VStack(spacing: 8) {
        Text(LocalizedStringKey(title))
          .appFont(.title, weight: .bold)
          .foregroundStyle(.primary)

        if let description {
          Text(LocalizedStringKey(description))
            .appFont(.bodySmall, weight: .regular)
            .foregroundStyle(AppThemePalette.textSecondary)
            .multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
        }
      }
      .accessibilityElement(children: .combine)

      if let actionTitle, let action {
        Button(action: action) {
          Label(LocalizedStringKey(actionTitle), systemImage: "arrow.clockwise")
        }
        .buttonStyle(AppThemeActionButtonStyle())
      }
    }
    .frame(maxWidth: 420)
    .padding(.horizontal, 32)
    .padding(.vertical, 24)
    .multilineTextAlignment(.center)
    .frame(maxWidth: .infinity, alignment: .center)
  }
}

private struct AppThemeActionButtonStyle: ButtonStyle {
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .appFont(.body, weight: .semibold)
      .foregroundStyle(.primary)
      .padding(.horizontal, 20)
      .frame(minHeight: 48)
      .contentShape(Capsule())
      .appSurface(radius: 999)
      .scaleEffect(configuration.isPressed ? 0.97 : 1)
      .opacity(configuration.isPressed ? 0.82 : 1)
      .animation(.easeOut(duration: 0.16), value: configuration.isPressed)
  }
}

private struct GameLibraryLoadingView: View {
  var body: some View {
    GeometryReader { geometry in
      AppPageScroll {
        VStack(spacing: 28) {
          RoundedRectangle(cornerRadius: 0)
            .fill(.quaternary)
            .frame(height: LibraryLayoutMetrics.heroHeight(for: geometry.size.height))
            .overlay(alignment: .bottomLeading) {
              VStack(alignment: .leading, spacing: 10) {
                placeholderLine(width: 210, height: 26)
                placeholderLine(width: 168, height: 14)
              }
              .padding(20)
              .padding(.bottom, 30)
            }

          ForEach(LibraryCollectionKind.allCases) { kind in
            VStack(alignment: .leading, spacing: 12) {
              placeholderLine(width: 132, height: 22)
                .padding(.horizontal, AppLayoutPolicy.pageInset)

              ScrollView(.horizontal) {
                HStack(spacing: 12) {
                  ForEach(0..<3, id: \.self) { _ in
                    RoundedRectangle(
                      cornerRadius: LibraryLayoutMetrics.posterCornerRadius,
                      style: .continuous
                    )
                    .fill(.quaternary)
                    .frame(
                      width: kind == .recent
                        ? LibraryLayoutMetrics.recentCardWidth
                        : LibraryLayoutMetrics.posterWidth,
                      height: kind == .recent
                        ? LibraryLayoutMetrics.recentCardHeight
                        : LibraryLayoutMetrics.posterHeight
                    )
                  }
                }
              }
              .contentMargins(
                .horizontal, AppLayoutPolicy.pageInset, for: .scrollContent
              )
              .scrollDisabled(true)
              .scrollIndicators(.hidden)
            }
          }
        }
        .padding(.bottom, 24)
      }
      .ignoresSafeArea(edges: .top)
    }
    .skeletonPulse(accessibilityLabel: "正在载入游戏库")
    .onAppear {
      IOSRuntimeTrace.state(
        domain: "library-ui",
        event: "skeletonPresented",
        payload: [
          "heroSections": 1,
          "shelfSections": .integer(Int64(LibraryCollectionKind.allCases.count)),
          "cardsPerShelf": 3,
        ],
        dimension: .frontend,
        importance: .key
      )
    }
  }

  private func placeholderLine(width: CGFloat, height: CGFloat) -> some View {
    Capsule()
      .fill(.quaternary)
      .frame(width: width, height: height)
  }
}
