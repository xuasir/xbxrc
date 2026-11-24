import SwiftUI

/// 沿 PC 色板与透明模式，按移动端参考图提高画布和内容表面的明度差。
enum AppThemePalette {
  static let brand = adaptive(dark: 0x95E8B1, light: 0x008746)
  static let canvas = adaptive(dark: 0x202E27, light: 0xF6FAF8)
  static let textPrimary = adaptive(dark: 0xFFFFFF, light: 0x202329)
  static let textSecondary = adaptive(dark: 0xCAD8D0, light: 0x525963)
  static let onBrand = adaptive(dark: 0x202E27, light: 0xFFFFFF)
  static let icon = adaptive(dark: 0xCADFD1, light: 0x525963)
  static let panel = adaptive(dark: 0x415D4E, light: 0xFFFFFF)
  static let filterTrack = adaptive(dark: 0x31433B, light: 0xE0E8E5)
  static let filterSelection = adaptive(dark: 0x607568, light: 0xFFFFFF)
  static let ambient = adaptive(dark: 0xBBDCC5, light: 0x8ACDB1)
  static let ambientAccent = adaptive(dark: 0x8ECDB2, light: 0x8ACDB1)
  static let ambientEdge = adaptive(dark: 0x007DB5, light: 0x007DB5)
  static let border = Color(
    uiColor: UIColor { traits in
      UIColor(white: traits.userInterfaceStyle == .dark ? 1 : 0, alpha: 0.08)
    })

  private static func adaptive(dark: UInt32, light: UInt32) -> Color {
    Color(
      uiColor: UIColor { traits in
        let rgb = traits.userInterfaceStyle == .dark ? dark : light
        return UIColor(
          red: CGFloat((rgb >> 16) & 255) / 255,
          green: CGFloat((rgb >> 8) & 255) / 255,
          blue: CGFloat(rgb & 255) / 255, alpha: 1)
      })
  }
}

struct AppThemeBackground: View {
  @Environment(\.colorScheme) private var colorScheme
  @Environment(\.appCanvasBounds) private var canvasBounds
  var canvasRegion: CGRect? = nil

  var body: some View {
    GeometryReader { geometry in
      let localBounds = canvasRegion ?? geometry.frame(in: .global)
      let bounds = canvasBounds ?? localBounds
      ZStack {
        AppThemePalette.canvas
        ZStack {
          if colorScheme == .dark {
            RadialGradient(
              colors: [AppThemePalette.ambient.opacity(0.16), .clear],
              center: UnitPoint(x: 0.55, y: 0.2), startRadius: 0,
              endRadius: max(bounds.width, bounds.height * 0.9)
            )
          }
          RadialGradient(
            colors: [
              AppThemePalette.ambientAccent.opacity(
                colorScheme == .dark ? 0.09 : 0.10),
              .clear,
            ], center: UnitPoint(x: 0.12, y: 0.28), startRadius: 0,
            endRadius: max(bounds.width * 0.8, bounds.height * 0.7)
          )
          RadialGradient(
            colors: [
              AppThemePalette.ambientEdge.opacity(
                colorScheme == .dark ? 0.06 : 0.055),
              .clear,
            ], center: UnitPoint(x: 0.9, y: 0.4), startRadius: 0,
            endRadius: max(bounds.width * 0.7, bounds.height * 0.65)
          )
        }
      }
      .frame(width: bounds.width, height: bounds.height)
      .offset(x: bounds.minX - localBounds.minX, y: bounds.minY - localBounds.minY)
    }
    .ignoresSafeArea()
    .allowsHitTesting(false)
    .accessibilityHidden(true)
  }
}

extension View {
  func appThemeCanvas() -> some View {
    modifier(AppThemeCanvasModifier())
      .appResponsiveTypography()
  }

  func appResponsiveTypography() -> some View {
    modifier(AppResponsiveTypographyModifier())
      .labelStyle(AppIconLabelStyle())
  }
}

private struct AppShellCanvasKey: EnvironmentKey {
  static let defaultValue = false
}

private struct AppCompactTypographyKey: EnvironmentKey {
  static let defaultValue = false
}

private struct AppTypographyManagedKey: EnvironmentKey {
  static let defaultValue = false
}

extension EnvironmentValues {
  @Entry var appCanvasBounds: CGRect? = nil

  var appShellOwnsCanvas: Bool {
    get { self[AppShellCanvasKey.self] }
    set { self[AppShellCanvasKey.self] = newValue }
  }

  var appUsesCompactTypography: Bool {
    get { self[AppCompactTypographyKey.self] }
    set { self[AppCompactTypographyKey.self] = newValue }
  }

  var appTypographyIsManaged: Bool {
    get { self[AppTypographyManagedKey.self] }
    set { self[AppTypographyManagedKey.self] = newValue }
  }
}

/// 按页面实际可用宽度统一字号密度，分屏与独立子页沿同一规则。
private struct AppResponsiveTypographyModifier: ViewModifier {
  @Environment(\.appTypographyIsManaged) private var isManaged
  @State private var usesCompactTypography = false

  func body(content: Content) -> some View {
    if isManaged {
      content
    } else {
      content
        .onGeometryChange(for: Bool.self) { geometry in
          geometry.size.width > 0 && geometry.size.width < 360
        } action: {
          usesCompactTypography = $0
        }
        .environment(\.appUsesCompactTypography, usesCompactTypography)
        .environment(\.appTypographyIsManaged, true)
    }
  }
}

private struct AppThemeCanvasModifier: ViewModifier {
  @Environment(\.appShellOwnsCanvas) private var shellOwnsCanvas
  @Environment(\.appInformationIsEmbedded) private var isEmbedded

  @ViewBuilder
  func body(content: Content) -> some View {
    if isEmbedded {
      content.appFont(.body).foregroundStyle(AppThemePalette.textPrimary)
    } else {
      content
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .appFont(.body)
        .foregroundStyle(AppThemePalette.textPrimary)
        .scrollContentBackground(.hidden)
        .scrollEdgeEffectHidden()
        .clipped()
        .background {
          // 原生导航容器拥有通顶画布，独立展示的内容自行提供背景。
          if !shellOwnsCanvas { AppThemeBackground() }
        }
    }
  }
}

struct AppRootView: View {
  @EnvironmentObject private var authStore: AuthStore
  let streamingStore: StreamingFeatureStore
  @State private var streamingPresented = false
  @State private var selection: AppSection = .hosts
  @State private var launchVisible = true

  init(streamingStore: StreamingFeatureStore) {
    self.streamingStore = streamingStore
    _streamingPresented = State(initialValue: streamingStore.isPresentingPlayer)
    #if DEBUG
      if ProcessInfo.processInfo.arguments.contains("--settings-ui-test") {
        _selection = State(initialValue: .settings)
        _launchVisible = State(initialValue: false)
      }
    #endif
  }

  var body: some View {
    ZStack {
      Group {
        if showsNavigation {
          AppNavigationShell(selection: $selection)
        } else {
          XboxLoginView(isBusy: authStore.isBusy, errorMessage: authStore.errorMessage) {
            Task { await authStore.retry() }
          }
          .appThemeCanvas()
        }
      }
      .opacity(streamingPresented ? 0 : 1)
      .allowsHitTesting(!streamingPresented)
      .accessibilityHidden(streamingPresented)

      if streamingPresented {
        StreamingPageRoot(store: streamingStore)
      }

      if launchVisible && !streamingPresented {
        LaunchExperienceView(isRestoring: authStore.phase == .restoring) {
          launchVisible = false
        }
      }
    }
    .onAppear {
      AppOrientationCoordinator.shared.sync(
        streamingPresented: streamingStore.isPresentingPlayer
      )
    }
    .onReceive(streamingStore.$state.map(\.presentsPlayer).removeDuplicates()) {
      streamingPresented = $0
    }
    .onChange(of: streamingPresented) { _, isPresenting in
      AppOrientationCoordinator.shared.sync(streamingPresented: isPresenting)
    }
  }

  private var showsNavigation: Bool {
    #if DEBUG
      // 设置夹具沿用独立入口，登录验收完整经过正式根视图。
      if ProcessInfo.processInfo.arguments.contains("--settings-ui-test")
        && !ProcessInfo.processInfo.arguments.contains("--signed-out-ui-test")
      {
        return true
      }
    #endif
    return authStore.isSignedIn
  }
}

enum AppSection: Int, CaseIterable, Identifiable {
  case hosts
  case library
  case my
  case controller
  case settings

  var id: Self { self }

  var title: String {
    switch self {
    case .hosts: "主机"
    case .library: "云游戏"
    case .my: "我的"
    case .controller: "控制器"
    case .settings: "设置"
    }
  }

  var systemImage: String {
    switch self {
    case .hosts: "desktopcomputer"
    case .library: "cloud"
    case .my: "person"
    case .controller: "gamecontroller"
    case .settings: "gearshape"
    }
  }

  var icon: AppIcon {
    switch self {
    case .hosts: .console
    case .library: .cloud
    case .my: .person
    case .controller: .xboxController
    case .settings: .settings
    }
  }
}

struct AppNavigationShell: View {
  @EnvironmentObject private var settings: AppSettingsStore
  @EnvironmentObject private var streamingStore: StreamingFeatureStore
  @Environment(\.colorScheme) private var colorScheme
  @Binding var selection: AppSection

  init(selection: Binding<AppSection>) {
    _selection = selection
  }

  var body: some View {
    GeometryReader { geometry in
      let globalFrame = geometry.frame(in: .global)
      let canvasBounds = CGRect(
        x: globalFrame.minX - geometry.safeAreaInsets.leading,
        y: globalFrame.minY - geometry.safeAreaInsets.top,
        width: geometry.size.width + geometry.safeAreaInsets.leading
          + geometry.safeAreaInsets.trailing,
        height: geometry.size.height + geometry.safeAreaInsets.top
          + geometry.safeAreaInsets.bottom)
      ZStack(alignment: .bottom) {
        TabView(selection: $selection) {
          ForEach(AppSection.allCases) { section in
            page(for: section)
              // 分页内部提供完整画布，宽窗侧栏与导航容器共享页面背景。
              .background { AppThemeBackground() }
              .environment(
                \.appPageIsActive,
                selection == section && !streamingStore.isPresentingPlayer
              )
              .tag(section)
              .toolbar(.hidden, for: .tabBar)
          }
        }
        .tabViewStyle(.tabBarOnly)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        PrimaryNavigation(selection: selection, onSelect: select)
          .padding(.horizontal, 8)
          .padding(.vertical, 8)
          .frame(maxWidth: 560)
          .background {
            Capsule(style: .continuous)
              .fill(.ultraThinMaterial)
              .overlay {
                Capsule(style: .continuous)
                  .fill(
                    AppThemePalette.canvas.opacity(
                      colorScheme == .dark ? 0.38 : 0.28))
              }
              .overlay {
                Capsule(style: .continuous)
                  .strokeBorder(AppThemePalette.border, lineWidth: 1)
              }
              .shadow(
                color: .black.opacity(colorScheme == .dark ? 0.18 : 0.07),
                radius: 16,
                y: 5)
          }
          .padding(.horizontal, AppLayoutPolicy.pageInset)
          .padding(
            .bottom,
            AppLayoutPolicy.navigationBottomPadding(
              safeAreaInset: geometry.safeAreaInsets.bottom)
          )
          .frame(maxWidth: .infinity)
      }
      .environment(\.appShellOwnsCanvas, true)
      .environment(\.appFloatingNavigationInset, AppLayoutPolicy.floatingNavigationInset)
      .environment(\.appCanvasBounds, canvasBounds)
      // 悬浮底栏叠在完整内容区上，页面和安全区共享一张连续画布。
      .background { AppThemeBackground() }
      .tint(AppThemePalette.brand)
      .appResponsiveTypography()
      .onChange(of: selection) { _, section in
        MobileInteractionFeedback.play(settings.mobile)
      }
    }
  }

  @ViewBuilder
  private func page(for section: AppSection) -> some View {
    let isActive = selection == section && !streamingStore.isPresentingPlayer
    switch section {
    case .hosts: HostListView(isActive: isActive)
    case .library: GameLibraryView(isActive: isActive)
    case .my: ProfileView(isActive: isActive)
    case .controller: ControllerView(isActive: isActive)
    case .settings: SettingsRootView()
    }
  }

  private func select(_ section: AppSection) {
    guard selection != section else { return }
    // 分页只更新归属，正文和底栏各自执行动效，保持原生标题栏的固定几何。
    var transaction = Transaction()
    transaction.animation = nil
    withTransaction(transaction) { selection = section }
  }
}

private struct PrimaryNavigation: View {
  let selection: AppSection
  let onSelect: (AppSection) -> Void
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @Namespace private var selectionAnimation

  var body: some View {
    HStack(spacing: 0) {
      ForEach(AppSection.allCases) { section in
        Button {
          onSelect(section)
        } label: {
          AppIconView(section.icon, size: 26, selected: selection == section)
            .foregroundStyle(
              selection == section
                ? AppThemePalette.textPrimary
                : AppThemePalette.icon
            )
            .frame(maxWidth: .infinity)
            .frame(height: 48)
            .background {
              if selection == section {
                AppSelectionBackdrop(outline: Capsule())
                  .matchedGeometryEffect(
                    id: "primary-selection", in: selectionAnimation
                  )
                  .padding(.horizontal, 3)
              }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(LocalizedStringKey(section.title)))
        .accessibilityAddTraits(selection == section ? .isSelected : [])
        .accessibilityIdentifier("primary-navigation-\(section)")
      }
    }
    .animation(
      reduceMotion ? .easeInOut(duration: 0.16) : .smooth(duration: 0.28), value: selection
    )
  }

}

private struct StreamingPageRoot: View {
  let store: StreamingFeatureStore

  var body: some View {
    StreamingPlayerView(store: store)
  }
}
