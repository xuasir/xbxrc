import SwiftUI

protocol AppSecondaryRoute: CaseIterable, Hashable, Identifiable {
  var title: String { get }
  var icon: AppIcon { get }
  var subtitle: String { get }
}

extension AppSecondaryRoute {
  var id: Self { self }
}

/// 底栏复用 PC 的 Xbox 导航资源，页面入口与操作采用 Fluent。
enum AppIcon: String {
  case console = "NavigationHosts"
  case cloud = "NavigationCloud"
  case person = "NavigationMy"
  case controller = "NavigationController"
  case xboxController = "NavigationXboxController"
  case settings = "NavigationSettings"
  case paintbrush = "IconPaintbrush"
  case video = "IconVideo"
  case network = "IconNetwork"
  case pulse = "IconPulse"
  case info = "IconInfo"
  case people = "IconPeople"
  case trophy = "IconTrophy"
  case chart = "IconChart"
  case photos = "IconPhotos"
  case sliders = "IconSliders"
  case vibration = "IconVibration"
  case touch = "IconTouch"
  case library = "IconLibrary"
  case search = "IconSearch"
  case star = "IconStar"
  case refresh = "IconRefresh"
  case signOut = "IconSignOut"
  case chevron = "IconChevron"
  case share = "IconShare"

  func assetName(selected: Bool) -> String {
    switch self {
    case .star:
      rawValue + (selected ? "Filled" : "")
    default:
      rawValue
    }
  }
}

struct AppIconView: View {
  let icon: AppIcon
  let size: CGFloat
  let selected: Bool

  init(_ icon: AppIcon, size: CGFloat = 24, selected: Bool = false) {
    self.icon = icon
    self.size = size
    self.selected = selected
  }

  var body: some View {
    Image(icon.assetName(selected: selected))
      .renderingMode(.template)
      .resizable().scaledToFit()
      .frame(width: size, height: size)
      .accessibilityHidden(true)
  }
}

enum AppLayoutPolicy {
  static let contentMaximumWidth: CGFloat = 1_040
  static let settingsMaximumWidth: CGFloat = 760
  static let pageInset: CGFloat = 24
  static let floatingNavigationInset: CGFloat = 88
  static let detailSpacing: CGFloat = 4
  static let itemSpacing: CGFloat = 8
  static let groupSpacing: CGFloat = 12
  static let sectionSpacing: CGFloat = 32
  static let rowInset: CGFloat = 12
  static let controllerArtworkMaximumWidth: CGFloat = 440
  static let secondarySidebarThreshold: CGFloat = 900
  static let secondarySidebarWidth: CGFloat = 220
  static let rowRadius: CGFloat = 6
  static let cardRadius: CGFloat = 8

  static func navigationBottomPadding(safeAreaInset: CGFloat) -> CGFloat {
    // 常规安全区计入屏幕下沿24pt；键盘显示时沿键盘顶边保留相同留白。
    if safeAreaInset > 48 { return pageInset }
    return pageInset - max(0, safeAreaInset)
  }
}

/// 动态组合和标题测量按应用语言选择资源目录，格式化继续使用当前 locale。
func appLocalizedString(_ key: String, locale: Locale) -> String {
  let language = Bundle.preferredLocalizations(
    from: Bundle.main.localizations, forPreferences: [locale.identifier]
  ).first
  let bundle =
    language.flatMap {
      Bundle.main.url(forResource: $0, withExtension: "lproj").flatMap(Bundle.init(url:))
    } ?? .main
  return bundle.localizedString(forKey: key, value: key, table: nil)
}

func appLocalizedFormat(_ key: String, locale: Locale, _ arguments: CVarArg...) -> String {
  String(format: appLocalizedString(key, locale: locale), locale: locale, arguments: arguments)
}

private struct AppFloatingNavigationInsetKey: EnvironmentKey {
  static let defaultValue: CGFloat = 0
}

extension EnvironmentValues {
  var appFloatingNavigationInset: CGFloat {
    get { self[AppFloatingNavigationInsetKey.self] }
    set { self[AppFloatingNavigationInsetKey.self] = newValue }
  }
}

extension View {
  /// 标题与操作叠在页面背景上，子页沿用系统返回和边缘返回手势。
  func appPageToolbar(
    _ title: String, showsBrand: Bool = false, showsBack: Bool = true,
    backIdentifier: String = "app-navigation-back",
    artwork: AppPageArtwork? = nil, onArtworkSuccess: ((URL) -> Void)? = nil,
    status: String? = nil
  ) -> some View {
    modifier(
      AppPageToolbarModifier(
        title: title, showsBrand: showsBrand, showsBack: showsBack,
        backIdentifier: backIdentifier,
        artwork: artwork, onArtworkSuccess: onArtworkSuccess, status: status)
    )
    .appResponsiveTypography()
  }

  /// 仅增加滚动末尾的避让距离，内容视口延伸到悬浮导航下方。
  func appFloatingNavigationClearance() -> some View {
    modifier(AppFloatingNavigationClearanceModifier())
  }

  func appPageTransition<Value: Equatable>(value: Value) -> some View {
    modifier(AppPageTransitionModifier(value: value))
  }

  func appFont(_ style: AppTextStyle, weight: Font.Weight = .regular) -> some View {
    modifier(AppFontModifier(style: style, weight: weight))
  }

  func appSurface(
    radius: CGFloat = AppLayoutPolicy.rowRadius, prominent: Bool = false,
    forceDark: Bool = false
  )
    -> some View
  {
    modifier(
      AppSurfaceModifier(
        outline: RoundedRectangle(cornerRadius: radius, style: .continuous),
        prominent: prominent, forceDark: forceDark))
  }

  /// 胶囊表面独立于触屏区域，紧凑视觉与44pt操作目标共用同一控件。
  func appCapsuleSurface(verticalInset: CGFloat = 0) -> some View {
    modifier(
      AppSurfaceModifier(
        outline: Capsule(), prominent: false, forceDark: false, verticalInset: verticalInset))
  }

  /// 筛选与下拉共用实色底槽，44pt操作区内保留相同的视觉留白。
  func appFilterTrackSurface() -> some View {
    background {
      Capsule().fill(AppThemePalette.filterTrack).padding(.vertical, 6)
    }
  }
}

private struct AppFloatingNavigationClearanceModifier: ViewModifier {
  @Environment(\.appFloatingNavigationInset) private var inset
  func body(content: Content) -> some View {
    content.contentMargins(.bottom, inset, for: .scrollContent)
  }
}

/// 原生 TabView 保留各页返回栈，切换时保持标题与背景的位置并淡入内容。
private struct AppPageTransitionModifier<Value: Equatable>: ViewModifier {
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  let value: Value
  @State private var hasAppeared = false
  @State private var visible = true

  func body(content: Content) -> some View {
    content
      .opacity(visible ? 1 : 0.35)
      .task(id: value) {
        guard hasAppeared else {
          hasAppeared = true
          return
        }
        var transaction = Transaction()
        transaction.animation = nil
        withTransaction(transaction) { visible = false }
        do { try await Task.sleep(for: .milliseconds(20)) } catch { return }
        withAnimation(.easeOut(duration: reduceMotion ? 0.16 : 0.24)) { visible = true }
      }
      .onDisappear { visible = true }
  }
}

/// 根页仅对正文淡入，顶部身份与背景保持固定；子页使用原生栈转场。
private struct AppRootContentTransitionModifier: ViewModifier {
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  let isRoot: Bool
  let isActive: Bool
  @State private var visible = true

  func body(content: Content) -> some View {
    content
      .opacity(visible ? 1 : 0.35)
      .task(id: isActive) {
        guard isRoot, isActive else {
          visible = true
          return
        }
        var transaction = Transaction()
        transaction.animation = nil
        withTransaction(transaction) { visible = false }
        do { try await Task.sleep(for: .milliseconds(20)) } catch { return }
        withAnimation(.easeOut(duration: reduceMotion ? 0.16 : 0.24)) { visible = true }
      }
      .onDisappear { visible = true }
  }
}

/// 透明渐变沿 PC 装配方式，移动端表面保持参考图的可读明度。
private struct AppSurfaceModifier<Outline: InsettableShape>: ViewModifier {
  @Environment(\.colorScheme) private var colorScheme
  @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
  let outline: Outline
  let prominent: Bool
  let forceDark: Bool
  var verticalInset: CGFloat = 0

  private var isDark: Bool { forceDark || colorScheme == .dark }

  private var darkSurfaceColors: [Color] {
    if reduceTransparency {
      return Array(repeating: Color(red: 65 / 255, green: 93 / 255, blue: 78 / 255), count: 2)
    }
    if prominent {
      return [
        Color(red: 52 / 255, green: 74 / 255, blue: 63 / 255).opacity(0.72),
        Color(red: 46 / 255, green: 67 / 255, blue: 57 / 255).opacity(0.68),
      ]
    }
    return [
      Color(red: 171 / 255, green: 208 / 255, blue: 184 / 255).opacity(0.08),
      Color(red: 145 / 255, green: 179 / 255, blue: 163 / 255).opacity(0.04),
    ]
  }

  func body(content: Content) -> some View {
    content.background {
      outline
        .fill(isDark && prominent ? AnyShapeStyle(.regularMaterial) : AnyShapeStyle(.clear))
        .environment(\.colorScheme, isDark ? .dark : .light)
        .overlay {
          outline
            .fill(
              LinearGradient(
                colors: isDark
                  ? darkSurfaceColors
                  : [
                    .white.opacity(prominent ? 0.98 : 0.96),
                    .white.opacity(prominent ? 0.96 : 0.92),
                  ],
                startPoint: .topLeading, endPoint: .bottomTrailing
              )
            )
        }
        .overlay {
          outline
            .strokeBorder(
              isDark ? .white.opacity(0.11) : .black.opacity(0.06), lineWidth: 1)
        }
        .shadow(
          color: .black.opacity(isDark ? 0.06 : 0.05),
          radius: prominent ? 10 : 4, y: prominent ? 4 : 2
        )
        .padding(.vertical, verticalInset)
    }
  }
}

/// 选中态共用轻主题色材质，形状决定导航和选项的轮廓。
struct AppSelectionBackdrop<Outline: Shape>: View {
  let outline: Outline
  @Environment(\.colorScheme) private var colorScheme
  @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

  var body: some View {
    Group {
      if reduceTransparency {
        outline.fill(AppThemePalette.canvas)
      } else {
        outline.fill(.ultraThinMaterial)
      }
    }
    .overlay {
      outline.fill(AppThemePalette.brand.opacity(colorScheme == .dark ? 0.16 : 0.12))
    }
    .allowsHitTesting(false)
    .accessibilityHidden(true)
  }
}

/// 数值沿 PC foundation，系统字体与 Dynamic Type 保持 Apple 平台行为。
enum AppTextStyle {
  case caption, bodySmall, body, section, title, hero

  var pointSize: CGFloat {
    switch self {
    case .caption: 12
    case .bodySmall: 13
    case .body: 14
    case .section: 18
    case .title: 18
    case .hero: 24
    }
  }

  var compactPointSize: CGFloat {
    switch self {
    case .caption: 11
    case .bodySmall: 12
    case .body: 13
    case .section: 16
    case .title: 17
    case .hero: 22
    }
  }

  var relativeTo: Font.TextStyle {
    switch self {
    case .caption: .footnote
    case .bodySmall: .subheadline
    case .body: .body
    case .section: .headline
    case .title: .title3
    case .hero: .title
    }
  }
}

private struct AppFontModifier: ViewModifier {
  @Environment(\.appUsesCompactTypography) private var usesCompactTypography
  @ScaledMetric private var pointSize: CGFloat
  let style: AppTextStyle
  let weight: Font.Weight

  init(style: AppTextStyle, weight: Font.Weight) {
    _pointSize = ScaledMetric(wrappedValue: style.pointSize, relativeTo: style.relativeTo)
    self.style = style
    self.weight = weight
  }

  func body(content: Content) -> some View {
    let ratio = usesCompactTypography ? style.compactPointSize / style.pointSize : 1
    content.font(.system(size: pointSize * ratio, weight: weight))
  }
}

private struct AppPageToolbarModifier: ViewModifier {
  @EnvironmentObject private var settings: AppSettingsStore
  @Environment(\.dismiss) private var dismiss
  @Environment(\.appUsesCompactTypography) private var usesCompactTypography
  @Environment(\.locale) private var locale
  @Environment(\.appPageIsActive) private var isActive
  let title: String
  let showsBrand: Bool
  let showsBack: Bool
  let backIdentifier: String
  let artwork: AppPageArtwork?
  let onArtworkSuccess: ((URL) -> Void)?
  let status: String?
  @State private var pageWidth: CGFloat = 390
  @ScaledMetric(relativeTo: .title3) private var rootTitleSize: CGFloat = 24
  @ScaledMetric(relativeTo: .headline) private var detailTitleSize: CGFloat = 18

  private var headingSize: CGFloat {
    if showsBrand || !showsBack {
      return min(
        usesCompactTypography ? 28 : 32,
        rootTitleSize * (usesCompactTypography ? 22 / 24 : 1))
    }
    return min(
      usesCompactTypography ? 22 : 24,
      detailTitleSize * (usesCompactTypography ? 17 / 18 : 1))
  }

  private var headingWidth: CGFloat {
    let font = UIFont.systemFont(ofSize: headingSize, weight: .semibold)
    let localizedTitle = appLocalizedString(title, locale: locale)
    let available = status == nil ? 160 : max(72, pageWidth - 200)
    return min(
      available, 160,
      ceil((localizedTitle as NSString).size(withAttributes: [.font: font]).width))
  }

  private var statusWidth: CGFloat {
    guard let status else { return 0 }
    let available = max(12, pageWidth - headingWidth - 182)
    let measured = ceil(
      (appLocalizedString(status, locale: locale) as NSString)
        .size(withAttributes: [.font: UIFont.systemFont(ofSize: 11)]).width)
    return measured <= available ? measured : 12
  }

  private var leadingWidth: CGFloat {
    (showsBrand || showsBack ? 52 : 0) + headingWidth
      + (status == nil ? 0 : 6 + statusWidth)
  }

  private var backgroundArtwork: AppPageArtwork? {
    guard let artwork else { return nil }
    switch artwork {
    case .avatar(nil), .game(nil): return nil
    case .cloudGame(let candidates): return candidates.isEmpty ? nil : artwork
    default: return artwork
    }
  }

  func body(content: Content) -> some View {
    content
      .modifier(AppRootContentTransitionModifier(isRoot: showsBrand, isActive: isActive))
      .navigationTitle("")
      .navigationBarTitleDisplayMode(.inline)
      .navigationBarBackButtonHidden(true)
      .toolbarRole(.editor)
      .toolbarBackground(.hidden, for: .navigationBar)
      .environment(\.appShellOwnsCanvas, true)
      .environment(\.appInformationIsEmbedded, false)
      .containerBackground(for: .navigation) {
        // 画布装配在原生导航容器中，状态栏与标题区使用同一张完整背景。
        AppThemeBackground()
          .overlay(alignment: .top) {
            if let artwork = backgroundArtwork {
              AppArtworkBackdrop(artwork: artwork, onSuccess: onArtworkSuccess)
            }
          }
          .ignoresSafeArea()
      }
      .onGeometryChange(for: CGFloat.self) {
        $0.size.width
      } action: {
        pageWidth = $0
      }
      .background {
        AppNavigationBackdrop(
          artwork: backgroundArtwork, onArtworkSuccess: onArtworkSuccess
        )
        .frame(width: 0, height: 0)
        if !showsBrand && showsBack {
          AppNavigationBackGesture().frame(width: 0, height: 0)
        }
      }
      .toolbar {
        ToolbarItem(placement: .topBarLeading) {
          HStack(spacing: 12) {
            if showsBrand {
              AppPageBrandMark()
            } else if showsBack {
              Button {
                MobileInteractionFeedback.play(settings.mobile)
                dismiss()
              } label: {
                AppIconView(.chevron)
                  .rotationEffect(.degrees(180))
                  .foregroundStyle(AppThemePalette.textPrimary)
                  .frame(width: 44, height: 44)
                  .contentShape(Rectangle())
              }
              .buttonStyle(.plain)
              .accessibilityLabel(Text("返回"))
              .accessibilityIdentifier(backIdentifier)
            }
            if let status {
              HStack(spacing: 6) {
                heading
                if statusWidth > 12 {
                  Text(LocalizedStringKey(status))
                    .font(.system(size: 11))
                    .foregroundStyle(AppThemePalette.textSecondary)
                    .lineLimit(1)
                    .frame(width: statusWidth, alignment: .leading)
                } else {
                  Image(systemName: "info.circle")
                    .font(.system(size: 12))
                    .foregroundStyle(AppThemePalette.textSecondary)
                    .accessibilityLabel(Text(LocalizedStringKey(status)))
                }
              }
              .accessibilityElement(children: .contain)
              .accessibilityIdentifier("page-title-status")
            } else {
              heading
            }
          }
          // 明确工具栏宽度，图片加载和目录状态更新共用同一头像槽位。
          .frame(width: leadingWidth, height: 44, alignment: .leading)
        }
        .sharedBackgroundVisibility(.hidden)
      }
  }

  private var heading: some View {
    Text(LocalizedStringKey(title))
      .font(.system(size: headingSize, weight: .semibold))
      .foregroundStyle(AppThemePalette.textPrimary)
      .lineLimit(1).minimumScaleFactor(0.75)
      .frame(width: headingWidth, alignment: .leading)
      .accessibilityAddTraits(.isHeader)
      .accessibilityIdentifier("current-page-title")
  }
}

/// 纯图标返回通过 dismiss 更新原生栈，边缘手势继续使用 UINavigationController 的原生转场。
private struct AppNavigationBackGesture: UIViewControllerRepresentable {
  func makeUIViewController(context: Context) -> BackGestureController {
    BackGestureController()
  }

  func updateUIViewController(_ controller: BackGestureController, context: Context) {
    controller.installGesture()
  }

  final class BackGestureController: UIViewController, UIGestureRecognizerDelegate {
    private weak var activeNavigation: UINavigationController?
    private weak var previousDelegate: (any UIGestureRecognizerDelegate)?

    override func viewWillAppear(_ animated: Bool) {
      super.viewWillAppear(animated)
      installGesture()
    }

    override func viewDidAppear(_ animated: Bool) {
      super.viewDidAppear(animated)
      installGesture()
    }

    override func viewDidDisappear(_ animated: Bool) {
      super.viewDidDisappear(animated)
      if let gesture = activeNavigation?.interactivePopGestureRecognizer,
        gesture.delegate === self
      {
        gesture.delegate = previousDelegate
      }
    }

    func installGesture() {
      guard let root = viewIfLoaded?.window?.rootViewController,
        let navigation = appNavigationController(containing: view, in: root),
        let gesture = navigation.interactivePopGestureRecognizer
      else { return }
      activeNavigation = navigation
      if gesture.delegate !== self {
        previousDelegate = gesture.delegate
        gesture.delegate = self
      }
      gesture.isEnabled = navigation.viewControllers.count > 1
    }

    func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
      guard let navigation = activeNavigation else { return false }
      return navigation.viewControllers.count > 1 && navigation.transitionCoordinator == nil
    }
  }
}

/// 原生标题栏覆盖滚动内容，背景按窗口坐标截取，连续衔接状态栏和页面画布。
private struct AppNavigationBackdrop: UIViewControllerRepresentable {
  @Environment(\.appCanvasBounds) private var canvasBounds
  @Environment(\.colorScheme) private var colorScheme
  let artwork: AppPageArtwork?
  let onArtworkSuccess: ((URL) -> Void)?

  func makeUIViewController(context: Context) -> BackdropController { BackdropController() }

  func updateUIViewController(_ controller: BackdropController, context: Context) {
    controller.canvasBounds = canvasBounds
    controller.artwork = artwork
    controller.onArtworkSuccess = onArtworkSuccess
    controller.colorScheme = colorScheme
    controller.updateBackdrop()
  }

  final class BackdropController: UIViewController {
    var canvasBounds: CGRect?
    var artwork: AppPageArtwork?
    var onArtworkSuccess: ((URL) -> Void)?
    var colorScheme: ColorScheme = .light
    private let hosting = UIHostingController(rootView: AnyView(EmptyView()))

    override func loadView() {
      view = UIView()
      view.isUserInteractionEnabled = false
    }

    override func viewWillAppear(_ animated: Bool) {
      super.viewWillAppear(animated)
      updateBackdrop()
    }

    override func viewDidLayoutSubviews() {
      super.viewDidLayoutSubviews()
      updateBackdrop()
    }

    override func viewDidDisappear(_ animated: Bool) {
      super.viewDidDisappear(animated)
      hosting.view.removeFromSuperview()
    }

    func updateBackdrop() {
      guard let window = view.window,
        let root = window.rootViewController,
        let navigation = appNavigationController(containing: view, in: root)
      else { return }
      let bar = navigation.navigationBar
      let bounds = canvasBounds ?? window.bounds
      let top = max(0, bar.convert(.zero, to: window).y - bounds.minY)
      let localRegion = CGRect(
        x: 0, y: -top, width: bar.bounds.width, height: bar.bounds.height + top)
      let region = bar.convert(localRegion, to: window)
      hosting.rootView = AnyView(
        ZStack(alignment: .topLeading) {
          AppThemeBackground(canvasRegion: bounds)
            .environment(\.appCanvasBounds, bounds)
            .frame(width: bounds.width, height: bounds.height)
            .overlay(alignment: .top) {
              if let artwork {
                AppArtworkBackdrop(artwork: artwork, onSuccess: onArtworkSuccess)
              }
            }
            .offset(x: bounds.minX - region.minX, y: bounds.minY - region.minY)
        }
        .frame(width: region.width, height: region.height, alignment: .topLeading)
        .clipped()
        .environment(\.colorScheme, colorScheme)
      )
      hosting.view.backgroundColor = .clear
      hosting.view.isUserInteractionEnabled = false
      hosting.view.accessibilityElementsHidden = true
      hosting.view.clipsToBounds = true
      // 标题和操作始终绘制在遮挡背景上方，兼容导航栏对子视图的重排。
      hosting.view.layer.zPosition = -1
      hosting.view.frame = localRegion
      if hosting.view.superview !== bar { bar.insertSubview(hosting.view, at: 0) }
    }
  }
}

/// SwiftUI 背景控制器按视图归属定位原生栈，页面切换和边缘返回共用定位规则。
@MainActor
private func appNavigationController(containing view: UIView, in controller: UIViewController)
  -> UINavigationController?
{
  var responder: UIResponder? = view
  while let current = responder {
    if let navigation = current as? UINavigationController { return navigation }
    responder = current.next
  }
  if let navigation = controller as? UINavigationController,
    let navigationView = navigation.viewIfLoaded, view.isDescendant(of: navigationView)
  {
    return navigation
  }
  for child in controller.children {
    if let navigation = appNavigationController(containing: view, in: child) {
      return navigation
    }
  }
  return nil
}

private struct AppPageBrandMark: View {
  var body: some View {
    Image("XboxLogo")
      .renderingMode(.template)
      .resizable().scaledToFit()
      // 图形与标题保持同一视觉量级，外层槽位维持五个根页的标题起点。
      .frame(width: 28, height: 28)
      .frame(width: 40, height: 40)
      .foregroundStyle(AppThemePalette.textPrimary)
      .accessibilityLabel(Text(verbatim: "Xbox"))
      .accessibilityIdentifier("page-xbox-logo")
  }
}

/// 二级菜单属于正文区域，原生标题栏始终占据完整窗口宽度。
struct AppSecondaryPageLayout<Sidebar: View, Content: View>: View {
  let usesSidebar: Bool
  @ViewBuilder var sidebar: () -> Sidebar
  @ViewBuilder var content: () -> Content

  var body: some View {
    HStack(spacing: 0) {
      if usesSidebar {
        sidebar().frame(width: AppLayoutPolicy.secondarySidebarWidth)
      }
      content().frame(maxWidth: .infinity, maxHeight: .infinity)
    }
  }
}

/// 二级选择和原生返回栈保持同一身份，宽窗口在标题栏下方显示分类列。
struct AdaptiveSectionNavigation<Route: AppSecondaryRoute, Overview: View, Detail: View>: View {
  let title: String
  @Binding var selection: Route?
  var compactRows = false
  var artwork: AppPageArtwork? = nil
  var featuredRoutes: Set<Route> = []
  var tileNavigation = false
  @ViewBuilder var overview: () -> Overview
  @ViewBuilder var detail: (Route) -> Detail
  @Environment(\.dynamicTypeSize) private var dynamicTypeSize

  var body: some View {
    GeometryReader { geometry in
      let usesSidebar =
        geometry.size.width >= AppLayoutPolicy.secondarySidebarThreshold
        && !dynamicTypeSize.isAccessibilitySize
      NavigationStack(
        path: Binding(
          get: { selection.map { [$0] } ?? [] },
          set: { selection = $0.last }
        )
      ) {
        AppSecondaryPageLayout(usesSidebar: usesSidebar, sidebar: { sidebar }) {
          AppPageScroll {
            VStack(alignment: .leading, spacing: 24) {
              overview()
              routeRows
            }
            .padding(.horizontal, AppLayoutPolicy.pageInset)
            .padding(.top, 24)
            .padding(.bottom, 32)
            .frame(maxWidth: AppLayoutPolicy.settingsMaximumWidth, alignment: .leading)
            .frame(maxWidth: .infinity)
          }
        }
        .appInformationTypography()
        .appThemeCanvas()
        .appPageToolbar(title, showsBrand: true, artwork: artwork)
        .navigationDestination(for: Route.self) { route in
          AppSecondaryPageLayout(usesSidebar: usesSidebar, sidebar: { sidebar }) {
            detail(route)
          }
          .appPageToolbar(
            route.title, showsBrand: usesSidebar,
            artwork: featuredRoutes.contains(route) ? artwork : nil)
        }
      }
      .frame(maxWidth: .infinity, maxHeight: .infinity)
      .onChange(of: usesSidebar, initial: true) { _, isWide in
        if isWide, selection == nil { selection = Route.allCases.first }
      }
    }
  }

  @ViewBuilder
  private var routeRows: some View {
    if tileNavigation {
      AppMediaGrid {
        ForEach(Array(Route.allCases)) { route in
          NavigationLink(value: route) {
            AppNavigationTile(
              title: route.title, subtitle: route.subtitle, icon: route.icon)
          }
          .buttonStyle(.plain)
          .accessibilityIdentifier("section-\(route.id)")
        }
      }
    } else {
      VStack(spacing: compactRows ? 12 : 16) {
        ForEach(Array(Route.allCases)) { route in
          NavigationLink(value: route) {
            AppNavigationRow(
              title: route.title, subtitle: route.subtitle,
              icon: route.icon, compact: compactRows)
          }
          .buttonStyle(.plain)
          .accessibilityIdentifier("section-\(route.id)")
        }
      }
    }
  }

  private var sidebar: some View {
    AppPageScroll {
      VStack(alignment: .leading, spacing: 12) {
        Text(LocalizedStringKey(title))
          .appFont(.section, weight: .semibold)
          .foregroundStyle(AppThemePalette.textPrimary)
          .frame(minHeight: 44)
        VStack(spacing: 4) {
          ForEach(Array(Route.allCases)) { route in
            Button {
              selection = route
            } label: {
              AppSidebarNavigationRow(
                title: route.title, selected: selection == route)
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(selection == route ? .isSelected : [])
            .accessibilityIdentifier("section-\(route.id)")
          }
        }
      }
      .padding(.horizontal, 16)
      .padding(.top, 24)
      .padding(.bottom, AppLayoutPolicy.pageInset)
    }
    .appInformationTypography()
    .accessibilityIdentifier("secondary-sidebar")
    .overlay(alignment: .trailing) {
      Rectangle().fill(AppThemePalette.border).frame(width: 1)
    }
  }
}

struct AppSidebarNavigationRow: View {
  let title: String
  let selected: Bool

  var body: some View {
    Text(LocalizedStringKey(title))
      .appFont(.body)
      .lineLimit(1)
      .minimumScaleFactor(0.85)
      .foregroundStyle(selected ? AppThemePalette.textPrimary : AppThemePalette.textSecondary)
      .padding(.horizontal, 12)
      .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
      .background {
        if selected { AppSelectionBackdrop(outline: RoundedRectangle(cornerRadius: 6)) }
      }
      .contentShape(Rectangle())
  }
}

struct AppNavigationRow: View {
  let title: String
  let subtitle: String
  let icon: AppIcon
  var compact = false
  @ScaledMetric(relativeTo: .body) private var iconSize: CGFloat = 24

  var body: some View {
    HStack(spacing: 16) {
      AppIconView(icon, size: iconSize)
        .foregroundStyle(AppThemePalette.icon)
        .frame(width: max(24, iconSize), height: max(24, iconSize))
      VStack(alignment: .leading, spacing: 4) {
        Text(LocalizedStringKey(title)).appFont(.body)
          .foregroundStyle(AppThemePalette.textPrimary)
        Text(LocalizedStringKey(subtitle)).appFont(compact ? .caption : .bodySmall)
          .foregroundStyle(AppThemePalette.textSecondary)
          .fixedSize(horizontal: false, vertical: true)
      }
      Spacer(minLength: 8)
      AppIconView(.chevron, size: 16)
        .foregroundStyle(AppThemePalette.textSecondary)
    }
    .padding(.horizontal, 16)
    .padding(.vertical, compact ? 8 : 12)
    .frame(maxWidth: .infinity, minHeight: compact ? 60 : 68, alignment: .leading)
    .contentShape(Rectangle())
    .appSurface(radius: 8)
    .accessibilityElement(children: .combine)
  }
}

/// PC 设置保持独立透明选项行，原生控件沿现有绑定更新。
struct AppSettingsForm<Content: View>: View {
  @ViewBuilder var content: () -> Content

  var body: some View {
    AppPageScroll {
      LazyVStack(alignment: .leading, spacing: AppLayoutPolicy.sectionSpacing) {
        ForEach(sections: content()) { section in
          VStack(alignment: .leading, spacing: AppLayoutPolicy.groupSpacing) {
            if !section.header.isEmpty {
              section.header
                .appFont(.section, weight: .semibold)
                .foregroundStyle(AppThemePalette.textPrimary)
                .accessibilityAddTraits(.isHeader)
                .padding(.horizontal, AppLayoutPolicy.rowInset)
            }
            Group {
              VStack(spacing: 0) {
                ForEach(subviews: section.content) { row in
                  VStack(
                    alignment: .leading, spacing: AppLayoutPolicy.detailSpacing
                  ) {
                    row
                      .appFont(.body)
                      .frame(
                        maxWidth: .infinity, alignment: .leading
                      )
                      .padding(
                        .horizontal,
                        row.containerValues.appSettingsRowOwnsPadding
                          ? 0 : AppLayoutPolicy.rowInset
                      )
                      .padding(
                        .vertical,
                        row.containerValues.appSettingsRowOwnsPadding
                          ? 0
                          : row.containerValues
                            .appSettingsRowIsInformation
                            ? 2 : 6
                      )
                      .frame(
                        minHeight: row.containerValues
                          .appSettingsRowIsInformation ? 0 : 44
                      )
                      .appSettingsRowSurface(
                        isInformation: row.containerValues
                          .appSettingsRowIsInformation
                      )
                    if let hint = row.containerValues.appSettingsHintText {
                      AppInlineHint(
                        hint,
                        identifier: row.containerValues
                          .appSettingsHintIdentifier
                      )
                      .padding(.horizontal, AppLayoutPolicy.rowInset)
                    }
                  }
                  .padding(
                    .bottom,
                    row.containerValues.appSettingsRowIsInformation ? 4 : 12
                  )
                }
              }
            }
            if !section.footer.isEmpty {
              section.footer
                .appFont(.caption)
                .foregroundStyle(AppThemePalette.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, AppLayoutPolicy.rowInset)
            }
          }
        }
      }
      .padding(AppLayoutPolicy.pageInset)
      .padding(.bottom, AppLayoutPolicy.pageInset)
      .frame(maxWidth: AppLayoutPolicy.settingsMaximumWidth, alignment: .leading)
      .frame(maxWidth: .infinity)
    }
    .accessibilityIdentifier("app-settings-scroll")
    .pickerStyle(.menu)
    .labeledContentStyle(AppSettingsValueStyle())
    .toggleStyle(AppSettingsToggleStyle())
    .buttonStyle(AppSettingsRowButtonStyle())
    .tint(AppThemePalette.brand)
    .appThemeCanvas()
    .appInformationTypography()
  }
}

struct AppSettingsPicker<Selection: Hashable, Content: View>: View {
  @EnvironmentObject private var settings: AppSettingsStore
  let title: String
  let selectedTitle: String
  @Binding var selection: Selection
  let content: () -> Content

  init(
    _ title: String, selection: Binding<Selection>, selectedTitle: String,
    @ViewBuilder content: @escaping () -> Content
  ) {
    self.title = title
    self.selectedTitle = selectedTitle
    _selection = selection
    self.content = content
  }

  var body: some View {
    Menu {
      Picker(title, selection: $selection, content: content)
        .labelsHidden()
        .pickerStyle(.inline)
    } label: {
      LabeledContent {
        HStack(spacing: 6) {
          Text(LocalizedStringKey(selectedTitle)).appFont(.caption)
          AppIconView(.chevron, size: 12).rotationEffect(.degrees(90))
        }
        .foregroundStyle(AppThemePalette.textSecondary)
      } label: {
        Text(LocalizedStringKey(title))
      }
      .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
      .padding(.horizontal, AppLayoutPolicy.rowInset)
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .accessibilityLabel(Text(LocalizedStringKey(title)))
    .accessibilityIdentifier("settings-picker-\(title)")
    .accessibilityValue(Text(LocalizedStringKey(selectedTitle)))
    .onChange(of: selection) { _, _ in MobileInteractionFeedback.play(settings.mobile) }
    .containerValue(\.appSettingsRowOwnsPadding, true)
  }
}

extension ContainerValues {
  @Entry var appSettingsRowOwnsPadding = false
  @Entry var appSettingsRowIsInformation = false
  @Entry var appSettingsHintText: String? = nil
  @Entry var appSettingsHintIdentifier: String? = nil
}

struct AppSettingsFact: View {
  let title: String
  let value: String
  init(_ title: String, value: String) {
    self.title = title
    self.value = value
  }
  var body: some View {
    LabeledContent {
      Text(LocalizedStringKey(value))
    } label: {
      Text(LocalizedStringKey(title))
    }.appSettingsInformation()
  }
}

extension View {
  /// 提示由表单装配到控件表面下方，共用同一行的间距和对齐。
  func appSettingsHint(_ text: String, identifier: String? = nil) -> some View {
    containerValue(\.appSettingsHintText, text)
      .containerValue(\.appSettingsHintIdentifier, identifier)
  }

  /// 只读事实与说明自然排布，交互项保留独立表面和完整触屏区域。
  func appSettingsInformation(ownsPadding: Bool = false) -> some View {
    containerValue(\.appSettingsRowIsInformation, true)
      .containerValue(\.appSettingsRowOwnsPadding, ownsPadding)
  }

  @ViewBuilder
  fileprivate func appSettingsRowSurface(isInformation: Bool) -> some View {
    if isInformation { self } else { appSurface(radius: AppLayoutPolicy.rowRadius) }
  }
}

private struct AppSettingsValueStyle: LabeledContentStyle {
  func makeBody(configuration: Configuration) -> some View {
    // 原生开关与文字值共用垂直中心，保持左右内容对齐。
    HStack(alignment: .center, spacing: 16) {
      configuration.label.appFont(.body)
        .foregroundStyle(AppThemePalette.textPrimary)
      Spacer(minLength: 8)
      configuration.content.appFont(.caption)
        .foregroundStyle(AppThemePalette.textSecondary)
        .multilineTextAlignment(.trailing)
        .fixedSize(horizontal: false, vertical: true)
    }
  }
}

/// 文字保持公共字号，开关单独缩放，操作区域保持 44pt。
private struct AppSettingsToggleStyle: ToggleStyle {
  @EnvironmentObject private var settings: AppSettingsStore
  var identifier = ""
  func makeBody(configuration: Configuration) -> some View {
    Button {
      configuration.isOn.toggle()
      MobileInteractionFeedback.play(settings.mobile)
    } label: {
      HStack(alignment: .center, spacing: 16) {
        configuration.label.appFont(.body)
          .foregroundStyle(AppThemePalette.textPrimary)
        Spacer(minLength: 8)
        Toggle(isOn: configuration.$isOn) { EmptyView() }
          .labelsHidden().toggleStyle(.switch)
          .fixedSize().scaleEffect(0.82)
          .frame(width: 44, height: 44)
          .allowsHitTesting(false)
      }
      .frame(maxWidth: .infinity, minHeight: 44)
      .padding(.horizontal, AppLayoutPolicy.rowInset)
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .accessibilityRepresentation {
      Toggle(isOn: configuration.$isOn) { configuration.label }.toggleStyle(.switch)
        .accessibilityIdentifier(identifier)
    }
    .containerValue(\.appSettingsRowOwnsPadding, true)
  }
}

struct AppSettingsRowButtonStyle: ButtonStyle {
  @EnvironmentObject private var settings: AppSettingsStore
  var inset: CGFloat = 0
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
      .padding(.horizontal, inset)
      .contentShape(Rectangle())
      .opacity(configuration.isPressed ? 0.6 : 1)
      .onChange(of: configuration.isPressed) { _, pressed in
        if pressed { MobileInteractionFeedback.play(settings.mobile) }
      }
  }
}

extension EnvironmentValues {
  @Entry var appPageIsActive = true
}

/// 卡片内边距属于交互控件，整张选项卡均可触发。
struct AppSettingsToggle: View {
  let title: String
  @Binding var isOn: Bool
  init(_ title: String, isOn: Binding<Bool>) {
    self.title = title
    _isOn = isOn
  }
  var body: some View {
    Toggle(LocalizedStringKey(title), isOn: $isOn)
      .toggleStyle(AppSettingsToggleStyle(identifier: "settings-toggle-\(title)"))
      .accessibilityIdentifier("settings-toggle-\(title)")
      .containerValue(\.appSettingsRowOwnsPadding, true)
  }
}

struct AppSettingsActionButton: View {
  let title: String
  var systemImage: String?
  var role: ButtonRole?
  let action: () -> Void
  init(
    _ title: String, systemImage: String? = nil, role: ButtonRole? = nil,
    action: @escaping () -> Void
  ) {
    self.title = title
    self.systemImage = systemImage
    self.role = role
    self.action = action
  }
  var body: some View {
    Button(role: role, action: action) {
      HStack {
        if let systemImage { Image(systemName: systemImage) }
        Text(LocalizedStringKey(title))
      }
    }
    .buttonStyle(AppSettingsRowButtonStyle(inset: AppLayoutPolicy.rowInset))
    .accessibilityIdentifier("settings-action-\(title)")
    .containerValue(\.appSettingsRowOwnsPadding, true)
  }
}

struct AppSettingsNavigationLink<Destination: View>: View {
  let title: String
  let destination: Destination
  init(_ title: String, destination: Destination) {
    self.title = title
    self.destination = destination
  }
  var body: some View {
    NavigationLink(destination: destination) { Text(LocalizedStringKey(title)) }
      .buttonStyle(AppSettingsRowButtonStyle(inset: AppLayoutPolicy.rowInset))
      .accessibilityIdentifier("settings-action-\(title)")
      .containerValue(\.appSettingsRowOwnsPadding, true)
  }
}

struct AppSettingsExternalLink: View {
  let title: String
  let destination: URL
  init(_ title: String, destination: URL) {
    self.title = title
    self.destination = destination
  }
  var body: some View {
    Link(LocalizedStringKey(title), destination: destination)
      .buttonStyle(AppSettingsRowButtonStyle(inset: AppLayoutPolicy.rowInset))
      .containerValue(\.appSettingsRowOwnsPadding, true)
  }
}
