import SwiftUI

/// 内容页面按语义分区，指标、事实与操作各自形成可辨的表面。
struct AppInformationPage<Content: View>: View {
  @ViewBuilder var content: () -> Content

  var body: some View {
    AppInformationScroll {
      VStack(alignment: .leading, spacing: AppLayoutPolicy.sectionSpacing) {
        AppAccountRequestNotice()
        ForEach(sections: content()) { section in
          VStack(alignment: .leading, spacing: AppLayoutPolicy.groupSpacing) {
            if !section.header.isEmpty {
              section.header.appFont(.section, weight: .semibold)
                .accessibilityAddTraits(.isHeader)
            }
            VStack(alignment: .leading, spacing: AppLayoutPolicy.itemSpacing) {
              ForEach(subviews: section.content) { row in
                row.frame(maxWidth: .infinity, alignment: .leading)
              }
            }
            if !section.footer.isEmpty {
              section.footer.appFont(.caption)
                .foregroundStyle(AppThemePalette.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            }
          }
        }
      }
      .padding(AppLayoutPolicy.pageInset).padding(.bottom, AppLayoutPolicy.pageInset)
      .frame(maxWidth: AppLayoutPolicy.contentMaximumWidth, alignment: .leading)
      .frame(maxWidth: .infinity)
    }
    .pickerStyle(.menu)
    .buttonStyle(AppInformationActionStyle())
    .tint(AppThemePalette.brand)
    .appThemeCanvas()
    .appInformationTypography()
  }
}

enum AppPageArtwork: Hashable {
  case avatar(URL?)
  case game(URL?)
  case cloudGame([URL])
  case console(String)
}

enum AppArtworkBackdropStyle {
  static let height: CGFloat = 380

  static func overlayStops(for colorScheme: ColorScheme) -> [Gradient.Stop] {
    [
      .init(
        color: AppThemePalette.canvas.opacity(colorScheme == .dark ? 0.36 : 0.52), location: 0),
      .init(
        color: AppThemePalette.canvas.opacity(colorScheme == .dark ? 0.52 : 0.70), location: 0.45),
      .init(color: AppThemePalette.canvas, location: 1),
    ]
  }
}

/// 页面和标题栏共用完整窗口的图片坐标，局部区域直接裁切同一张通顶背景。
struct AppPageBackdrop: View {
  @Environment(\.appCanvasBounds) private var canvasBounds
  let artwork: AppPageArtwork?
  var canvasRegion: CGRect? = nil
  var onArtworkSuccess: ((URL) -> Void)? = nil

  var body: some View {
    GeometryReader { geometry in
      let region = canvasRegion ?? geometry.frame(in: .global)
      let bounds = canvasBounds ?? region
      ZStack(alignment: .topLeading) {
        AppThemeBackground(canvasRegion: region)
        if let artwork {
          AppArtworkBackdrop(artwork: artwork, onSuccess: onArtworkSuccess)
            .frame(width: bounds.width, height: AppArtworkBackdropStyle.height)
            .offset(x: bounds.minX - region.minX, y: bounds.minY - region.minY)
        }
      }
      .frame(width: geometry.size.width, height: geometry.size.height, alignment: .topLeading)
    }
    .ignoresSafeArea()
    .allowsHitTesting(false).accessibilityHidden(true)
  }
}

/// 图片与主题画布渐变融合，延伸至安全区；背景专门表达身份。
struct AppArtworkBackdrop: View {
  @Environment(\.colorScheme) private var colorScheme
  let artwork: AppPageArtwork
  var onSuccess: ((URL) -> Void)? = nil
  var height: CGFloat = AppArtworkBackdropStyle.height
  var body: some View {
    GeometryReader { geometry in
      ZStack {
        AppThemePalette.canvas
        Group {
          switch artwork {
          case .console(let asset):
            Image(asset).resizable().scaledToFit()
              .padding(.leading, geometry.size.width * 0.28)
              .padding(.top, 48)
              .frame(height: min(300, geometry.size.height), alignment: .bottom)
              .frame(maxHeight: .infinity, alignment: .bottom)
          case .avatar(let url), .game(let url):
            SharedRemoteImage(url: url, onSuccess: onSuccess) { image in
              image.resizable().scaledToFill()
            } placeholder: { _ in
              Image(
                systemName: isAvatar
                  ? "person.crop.circle.fill" : "gamecontroller.fill"
              )
              .resizable().scaledToFit().padding(64)
              .foregroundStyle(AppThemePalette.brand.opacity(0.3))
            }
            .blur(radius: isAvatar ? 12 : 2)
          case .cloudGame(let candidates):
            CloudGameRemoteImage(
              candidates: candidates, contentMode: .fill, onSuccess: onSuccess)
          }
        }
        .id(artwork)
        .frame(width: geometry.size.width, height: geometry.size.height)
        .clipped()
        LinearGradient(
          stops: AppArtworkBackdropStyle.overlayStops(for: colorScheme),
          startPoint: .top, endPoint: .bottom)
      }
    }
    .frame(height: height)
    .mask {
      LinearGradient(
        stops: [
          .init(color: .white, location: 0), .init(color: .white, location: 0.65),
          .init(color: .clear, location: 1),
        ], startPoint: .top, endPoint: .bottom)
    }
    .allowsHitTesting(false).accessibilityHidden(true)
  }

  private var isAvatar: Bool {
    if case .avatar = artwork { return true }
    return false
  }
}

/// 提示按文字高度贴附相关操作，参与同一信息组。
struct AppInlineHint: View {
  let text: String
  let identifier: String

  init(_ text: String, identifier: String? = nil) {
    self.text = text
    self.identifier = identifier ?? text
  }

  var body: some View {
    Label {
      Text(LocalizedStringKey(text))
        .appFont(.caption)
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityIdentifier("settings-hint-\(identifier)")
    } icon: {
      Image(systemName: "info.circle").appFont(.caption)
    }
    .labelStyle(AppIconLabelStyle())
    .foregroundStyle(AppThemePalette.textSecondary)
    .frame(maxWidth: .infinity, alignment: .leading)
  }
}

/// 操作说明按短步骤排列，窄屏与大字号按完整步骤换行。
struct AppInstructionSteps: View {
  let steps: [String]

  var body: some View {
    AppWrappingLayout(spacing: 12, lineSpacing: 8) {
      ForEach(Array(steps.enumerated()), id: \.offset) { index, step in
        HStack(spacing: 6) {
          Text("\(index + 1)")
            .appFont(.caption, weight: .semibold)
            .foregroundStyle(AppThemePalette.brand)
            .frame(minWidth: 20, minHeight: 20)
            .background(AppThemePalette.brand.opacity(0.12), in: Circle())
          Text(LocalizedStringKey(step))
            .appFont(.caption)
            .foregroundStyle(AppThemePalette.textSecondary)
        }
        .accessibilityElement(children: .combine)
      }
    }
  }
}

struct AppMetric: View {
  let title: String
  let value: String
  var identifier: String?
  var compact = false
  var context: String? = nil
  var hint: String = ""

  var body: some View {
    VStack(alignment: .leading, spacing: AppLayoutPolicy.detailSpacing) {
      Text(value).appFont(.section, weight: .semibold).monospacedDigit()
        .foregroundStyle(AppThemePalette.textPrimary)
        .fixedSize(horizontal: compact, vertical: true)
      Text(LocalizedStringKey(title)).appFont(.caption)
        .foregroundStyle(AppThemePalette.textSecondary)
        .fixedSize(horizontal: compact, vertical: true)
      if let context {
        Text(LocalizedStringKey(context)).appFont(.caption)
          .foregroundStyle(AppThemePalette.textSecondary)
          .fixedSize(horizontal: false, vertical: true)
      }
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(Text(LocalizedStringKey(title)))
    .accessibilityValue(context.map { "\(value)，\($0)" } ?? value)
    .accessibilityHint(Text(LocalizedStringKey(hint)))
    .accessibilityIdentifier(identifier ?? "information-metric-\(title)")
  }
}

struct AppMetricGrid<Content: View>: View {
  @Environment(\.dynamicTypeSize) private var textSize
  @ViewBuilder var content: () -> Content
  var body: some View {
    LazyVGrid(
      columns: textSize.isAccessibilitySize
        ? [GridItem(.flexible(), alignment: .topLeading)]
        : [GridItem(.adaptive(minimum: 132), spacing: 16, alignment: .topLeading)],
      alignment: .leading, spacing: 16, content: content
    )
    .padding(16).appSurface(radius: AppLayoutPolicy.cardRadius)
  }
}

struct AppRefreshButton: View {
  @Environment(\.appIsPullRefreshing) private var isPullRefreshing
  let title: String
  var isRefreshing: Bool
  let action: @MainActor () async -> Void
  @State private var running = false

  init(_ title: String, isRefreshing: Bool = false, action: @escaping @MainActor () async -> Void) {
    self.title = title
    self.isRefreshing = isRefreshing
    self.action = action
  }

  var body: some View {
    Button {
      guard !running && !isRefreshing && !isPullRefreshing else { return }
      running = true
      Task {
        await action()
        running = false
      }
    } label: {
      Group {
        if (running || isRefreshing) && !isPullRefreshing {
          ProgressView()
        } else {
          AppIconView(.refresh, size: 22)
        }
      }.frame(width: 44, height: 44)
    }
    .buttonStyle(.plain).disabled(running || isRefreshing || isPullRefreshing)
    .foregroundStyle(AppThemePalette.textPrimary)
    .accessibilityLabel(Text(LocalizedStringKey(title)))
  }
}

/// 事实标签与值纵向排列，卡片明确同组信息的边界。
struct AppFactGroup: View {
  @Environment(\.dynamicTypeSize) private var textSize
  let facts: [(String, String)]
  var usesSurface = true

  var body: some View {
    LazyVGrid(
      columns: textSize.isAccessibilitySize
        ? [GridItem(.flexible(), alignment: .topLeading)]
        : [GridItem(.adaptive(minimum: 132), spacing: 16, alignment: .topLeading)],
      alignment: .leading, spacing: 12
    ) {
      ForEach(Array(facts.enumerated()), id: \.offset) { _, fact in
        VStack(alignment: .leading, spacing: 4) {
          Text(LocalizedStringKey(fact.0)).appFont(.caption)
            .foregroundStyle(AppThemePalette.textSecondary)
          Text(LocalizedStringKey(fact.1)).appFont(.body)
            .foregroundStyle(AppThemePalette.textPrimary)
            .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
      }
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .padding(usesSurface ? 16 : 0)
    .background {
      if usesSurface { Color.clear.appSurface(radius: AppLayoutPolicy.cardRadius) }
    }
  }
}

struct AppWrappingLayout: Layout {
  var spacing: CGFloat = 8
  var lineSpacing: CGFloat = 4

  private func arrangement(width: CGFloat, subviews: Subviews) -> (CGSize, [CGRect]) {
    var x: CGFloat = 0
    var y: CGFloat = 0
    var rowHeight: CGFloat = 0
    var usedWidth: CGFloat = 0
    var frames: [CGRect] = []
    for subview in subviews {
      let size = subview.sizeThatFits(ProposedViewSize(width: width, height: nil))
      if x > 0 && x + size.width > width {
        y += rowHeight + lineSpacing
        x = 0
        rowHeight = 0
      }
      frames.append(CGRect(origin: CGPoint(x: x, y: y), size: size))
      usedWidth = max(usedWidth, x + size.width)
      x += size.width + spacing
      rowHeight = max(rowHeight, size.height)
    }
    return (CGSize(width: usedWidth, height: y + rowHeight), frames)
  }

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    arrangement(width: proposal.width ?? 760, subviews: subviews).0
  }

  func placeSubviews(
    in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()
  ) {
    let frames = arrangement(width: bounds.width, subviews: subviews).1
    for (subview, frame) in zip(subviews, frames) {
      subview.place(
        at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY),
        proposal: ProposedViewSize(frame.size))
    }
  }
}

struct AppMediaGrid<Content: View>: View {
  @ViewBuilder var content: () -> Content
  var body: some View {
    LazyVGrid(
      columns: [GridItem(.adaptive(minimum: 132), spacing: 16, alignment: .topLeading)],
      alignment: .leading, spacing: 24, content: content)
  }
}

/// 筛选保持单行，窄窗通过横向滚动显露完整选项。
struct AppContentToolbar<Content: View>: View {
  @ViewBuilder var content: () -> Content
  var body: some View {
    ScrollView(.horizontal) {
      HStack(spacing: 8, content: content)
    }
    .scrollIndicators(.hidden)
    .frame(maxWidth: .infinity, alignment: .leading)
  }
}

/// 预留选中字重的宽度，切换时保持文字与相邻控件的位置稳定。
private struct AppSelectionText: View {
  let title: String
  let selected: Bool
  let style: AppTextStyle

  var body: some View {
    ZStack {
      Text(LocalizedStringKey(title)).appFont(style, weight: .semibold)
        .hidden().accessibilityHidden(true)
      Text(LocalizedStringKey(title))
        .appFont(style, weight: selected ? .semibold : .regular)
    }
    .fixedSize(horizontal: true, vertical: false)
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(Text(LocalizedStringKey(title)))
  }
}

/// 内容分类统一为透明文字Tab，选中下划线与完整无障碍名称共用。
struct AppTextTabs<Selection: Hashable>: View {
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @Namespace private var selectionAnimation
  @Binding var selection: Selection
  let values: [Selection]
  let title: (Selection) -> String
  let identifier: String
  var buttonPrefix: String = "content-tab"
  var inset: CGFloat = AppLayoutPolicy.pageInset

  var body: some View {
    ScrollViewReader { proxy in
      ScrollView(.horizontal) {
        HStack(spacing: AppContentControlMetrics.tabSpacing) {
          ForEach(values, id: \.self) { value in
            Button {
              selection = value
            } label: {
              AppSelectionText(title: title(value), selected: selection == value, style: .body)
                .foregroundStyle(
                  selection == value
                    ? AppThemePalette.brand : AppThemePalette.textSecondary
                )
                .padding(.vertical, AppContentControlMetrics.verticalPadding)
                .frame(minWidth: AppContentControlMetrics.minimumHitSize, minHeight: 48)
                .overlay(alignment: .bottom) {
                  if selection == value {
                    Capsule().fill(AppThemePalette.brand)
                      .frame(height: AppContentControlMetrics.underlineHeight)
                      .matchedGeometryEffect(id: "content-selection", in: selectionAnimation)
                  }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain).id(value)
            .accessibilityIdentifier("\(buttonPrefix)-\(value)")
            .accessibilityAddTraits(selection == value ? .isSelected : [])
          }
        }.padding(.horizontal, inset)
          .animation(
            AppContentControlMetrics.animation(reduceMotion: reduceMotion), value: selection)
      }
      .scrollIndicators(.hidden)
      .fixedSize(horizontal: false, vertical: true).frame(minHeight: 52)
      .accessibilityIdentifier(identifier)
      .onAppear { proxy.scrollTo(selection, anchor: .center) }
      .onChange(of: selection) { _, value in
        withAnimation(AppContentControlMetrics.animation(reduceMotion: reduceMotion)) {
          proxy.scrollTo(value, anchor: .center)
        }
      }
    }
  }
}

struct AppInformationActionStyle: ButtonStyle {
  func makeBody(configuration: Configuration) -> some View {
    configuration.label.appFont(.body, weight: .semibold)
      .foregroundStyle(AppThemePalette.brand)
      .padding(.horizontal, 12).padding(.vertical, 8)
      .frame(minHeight: 44, alignment: .leading)
      .contentShape(Rectangle())
      .appSurface(radius: AppLayoutPolicy.rowRadius)
      .opacity(configuration.isPressed ? 0.7 : 1)
  }
}

struct AppNavigationTile: View {
  let title: String
  let subtitle: String
  let icon: AppIcon
  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      AppIconView(icon).foregroundStyle(AppThemePalette.icon)
      VStack(alignment: .leading, spacing: 6) {
        Text(LocalizedStringKey(title)).appFont(.body, weight: .semibold)
        Text(LocalizedStringKey(subtitle)).appFont(.caption)
          .foregroundStyle(AppThemePalette.textSecondary)
          .fixedSize(horizontal: false, vertical: true)
      }
    }
    .padding(16).frame(maxWidth: .infinity, minHeight: 128, alignment: .topLeading)
    .contentShape(Rectangle()).appSurface(radius: 8)
    .accessibilityElement(children: .combine)
  }
}

private struct AppInformationSidebarKey: EnvironmentKey {
  static let defaultValue = false
}

private struct AppInformationEmbeddedKey: EnvironmentKey {
  static let defaultValue = false
}

extension EnvironmentValues {
  var appInformationIsEmbedded: Bool {
    get { self[AppInformationEmbeddedKey.self] }
    set { self[AppInformationEmbeddedKey.self] = newValue }
  }
  var appInformationIsSidebar: Bool {
    get { self[AppInformationSidebarKey.self] }
    set { self[AppInformationSidebarKey.self] = newValue }
  }
}

extension View {
  func appInformationTypography() -> some View {
    appResponsiveTypography()
  }
}

/// 我的分页正文共享外层纵向滚动，独立详情仍持有自己的滚动区域。
struct AppInformationScroll<Content: View>: View {
  @Environment(\.appInformationIsEmbedded) private var isEmbedded
  @ViewBuilder var content: () -> Content
  var body: some View {
    if isEmbedded {
      content()
    } else {
      AppPageScroll { content() }.accessibilityIdentifier("app-information-scroll")
    }
  }
}

/// 页面滚动视口占满窗口，底部留白只参与滚动范围。
struct AppPageScroll<Content: View>: View {
  @Environment(\.appFloatingNavigationInset) private var navigationInset
  var bottomClearance: CGFloat = 0
  @ViewBuilder var content: () -> Content

  var body: some View {
    ScrollView { content() }
      .contentMargins(.bottom, navigationInset + bottomClearance, for: .scrollContent)
      .clipped()
  }
}

extension EnvironmentValues {
  @Entry var appIsPullRefreshing = false
}

/// 原生下拉负责这一轮刷新反馈，分区、重试和分页共享当前页面的刷新范围。
private struct AppRefreshableModifier: ViewModifier {
  @Environment(\.appIsPullRefreshing) private var parentIsRefreshing
  @State private var isRefreshing = false
  let action: @MainActor () async -> Void

  func body(content: Content) -> some View {
    content
      .environment(\.appIsPullRefreshing, parentIsRefreshing || isRefreshing)
      .refreshable {
        guard !isRefreshing else { return }
        isRefreshing = true
        defer { isRefreshing = false }
        await action()
      }
  }
}

extension View {
  func appRefreshable(action: @escaping @MainActor () async -> Void) -> some View {
    modifier(AppRefreshableModifier(action: action))
  }
}

/// 搜索位于局部工具区，展开后的输入表面随实际字段宽度呈现。
struct AppContentSearch: View {
  @Binding var text: String
  let prompt: String
  @State private var isExpanded = false
  @FocusState private var focused: Bool

  var body: some View {
    Group {
      if isExpanded || !text.isEmpty {
        HStack(spacing: 8) {
          Button {
            isExpanded.toggle()
            focused = isExpanded
          } label: {
            AppIconView(.search, size: 22).frame(width: 44, height: 44).contentShape(
              Rectangle())
          }
          .buttonStyle(.plain).accessibilityLabel(Text(LocalizedStringKey(prompt)))
          .accessibilityIdentifier("content-search-toggle")
          TextField(LocalizedStringKey(prompt), text: $text)
            .textInputAutocapitalization(.never).autocorrectionDisabled()
            .focused($focused).submitLabel(.search).onSubmit { focused = false }
            .accessibilityIdentifier("content-search-field")
          if !text.isEmpty {
            Button {
              text = ""
            } label: {
              Image(systemName: "xmark.circle.fill").frame(width: 44, height: 44)
            }.buttonStyle(.plain).accessibilityLabel("清除搜索")
          }
        }
        .padding(.horizontal, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .appCapsuleSurface(verticalInset: 4)
      } else {
        Button {
          isExpanded = true
          focused = true
        } label: {
          HStack(spacing: 8) {
            AppIconView(.search, size: 22)
            Text(LocalizedStringKey(prompt)).foregroundStyle(
              AppThemePalette.textSecondary)
          }
          .padding(.horizontal, 12)
          .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
          .contentShape(Rectangle())
          .appCapsuleSurface(verticalInset: 4)
        }
        .buttonStyle(.plain).accessibilityIdentifier("content-search-toggle")
      }
    }
    .appFont(.body).accessibilityElement(children: .contain)
  }
}

/// 下拉与筛选共用底槽和高度，当前值与箭头共同表达操作。
struct AppInformationPicker<Selection: Hashable, Content: View>: View {
  @EnvironmentObject private var settings: AppSettingsStore
  let title: String
  @Binding var selection: Selection
  let selectedTitle: String
  @ViewBuilder var content: () -> Content

  init(
    _ title: String, selection: Binding<Selection>, selectedTitle: String,
    @ViewBuilder content: @escaping () -> Content
  ) {
    self.title = title
    _selection = selection
    self.selectedTitle = selectedTitle
    self.content = content
  }

  var body: some View {
    Menu {
      // inline 将所有选项直接放在第一层菜单，点击即可切换。
      Picker(title, selection: $selection, content: content).pickerStyle(.inline)
    } label: {
      HStack(spacing: AppContentControlMetrics.labelSpacing) {
        Text(LocalizedStringKey(selectedTitle)).appFont(.bodySmall, weight: .semibold)
        AppIconView(.chevron, size: 10).rotationEffect(.degrees(90))
      }
      .foregroundStyle(AppThemePalette.textPrimary)
      .padding(.horizontal, AppContentControlMetrics.horizontalPadding)
      .padding(.vertical, AppContentControlMetrics.verticalPadding)
      .frame(minHeight: AppContentControlMetrics.minimumHitSize).contentShape(Rectangle())
      .appFilterTrackSurface()
    }
    .fixedSize(horizontal: true, vertical: false).buttonStyle(.plain)
    .accessibilityLabel(Text(LocalizedStringKey(title)))
    .accessibilityValue(Text(LocalizedStringKey(selectedTitle)))
    .accessibilityIdentifier("settings-picker-\(title)")
    .onChange(of: selection) { _, _ in MobileInteractionFeedback.play(settings.mobile) }
  }
}

/// 互斥筛选共用底槽，选中胶囊内缩2pt；视觉表面与44pt触屏区分别确定。
struct AppSegmentedFilter<Selection: Hashable>: View {
  @EnvironmentObject private var settings: AppSettingsStore
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @Namespace private var selectionAnimation
  @Binding var selection: Selection
  let values: [Selection]
  let title: (Selection) -> String
  let identifier: String
  let label: String
  var fillsWidth = true

  var body: some View {
    ScrollViewReader { proxy in
      ViewThatFits(in: .horizontal) {
        segments
        ScrollView(.horizontal) { segments.fixedSize(horizontal: true, vertical: false) }
          .scrollIndicators(.hidden)
      }
      .accessibilityElement(children: .contain)
      .accessibilityLabel(Text(LocalizedStringKey(label)))
      .accessibilityIdentifier(identifier)
      .onAppear { proxy.scrollTo(selection, anchor: .center) }
      .onChange(of: selection) { _, value in
        MobileInteractionFeedback.play(settings.mobile)
        withAnimation(AppContentControlMetrics.animation(reduceMotion: reduceMotion)) {
          proxy.scrollTo(value, anchor: .center)
        }
      }
    }
  }

  private var segments: some View {
    HStack(spacing: 0) {
      ForEach(values, id: \.self) { value in
        Button {
          selection = value
        } label: {
          AppSelectionText(title: title(value), selected: selection == value, style: .bodySmall)
            .foregroundStyle(AppThemePalette.textPrimary)
            .padding(.horizontal, AppContentControlMetrics.horizontalPadding)
            .padding(.vertical, AppContentControlMetrics.verticalPadding)
            .frame(
              minWidth: AppContentControlMetrics.minimumHitSize,
              maxWidth: fillsWidth ? .infinity : nil,
              minHeight: AppContentControlMetrics.minimumHitSize
            )
            .appFilterSelectionSurface(
              isSelected: selection == value, namespace: selectionAnimation
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain).id(value)
        .accessibilityIdentifier("\(identifier)-\(value)")
        .accessibilityAddTraits(selection == value ? .isSelected : [])
      }
    }
    .padding(.horizontal, AppContentControlMetrics.selectionInset)
    .appFilterTrackSurface()
    .animation(AppContentControlMetrics.animation(reduceMotion: reduceMotion), value: selection)
  }
}

/// 工具动作与选择器共用胶囊尺寸，品牌文字表达可执行操作。
struct AppContentCapsuleButtonStyle: ButtonStyle {
  func makeBody(configuration: Configuration) -> some View {
    configuration.label.appFont(.bodySmall, weight: .semibold)
      .foregroundStyle(AppThemePalette.brand)
      .padding(.horizontal, AppContentControlMetrics.horizontalPadding)
      .padding(.vertical, AppContentControlMetrics.verticalPadding)
      .frame(
        minWidth: AppContentControlMetrics.minimumHitSize,
        minHeight: AppContentControlMetrics.minimumHitSize
      )
      .appFilterTrackSurface().contentShape(Rectangle())
      .opacity(configuration.isPressed ? 0.7 : 1)
  }
}

/// 分页作为列表末尾动作居中，真实等待沿同一个按钮反馈。
struct AppLoadMoreButton: View {
  @Environment(\.appIsPullRefreshing) private var isPullRefreshing
  var title = "载入更多"
  let isLoading: Bool
  let action: @MainActor () async -> Void
  @State private var running = false

  var body: some View {
    Button {
      guard !running && !isLoading && !isPullRefreshing else { return }
      running = true
      Task {
        await action()
        running = false
      }
    } label: {
      HStack(spacing: 8) {
        if (isLoading || running) && !isPullRefreshing { ProgressView().controlSize(.small) }
        Text(LocalizedStringKey((isLoading || running) && !isPullRefreshing ? "正在载入…" : title))
      }
      .appFont(.bodySmall, weight: .semibold).foregroundStyle(AppThemePalette.brand)
      .padding(.horizontal, 16).frame(minHeight: 44)
      .appCapsuleSurface(verticalInset: 6).contentShape(Rectangle())
    }
    .buttonStyle(.plain).disabled(isLoading || running || isPullRefreshing)
    .accessibilityIdentifier("content-load-more")
    .frame(maxWidth: .infinity)
  }
}

struct AppCompactPlayButtonStyle: ButtonStyle {
  func makeBody(configuration: Configuration) -> some View {
    configuration.label.appFont(.body, weight: .semibold).foregroundStyle(AppThemePalette.onBrand)
      .padding(.horizontal, 16).frame(minWidth: 112, minHeight: 44)
      .background(AppThemePalette.brand, in: RoundedRectangle(cornerRadius: 6))
      .opacity(configuration.isPressed ? 0.75 : 1)
  }
}
