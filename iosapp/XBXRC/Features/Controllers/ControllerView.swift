import CoreHaptics
import GameController
import SwiftUI

private enum ControllerRoute: AppSecondaryRoute {
  case connection, touch, sticks, mapping, vibration, menu
  var title: String {
    switch self {
    case .connection: "连接与测试"
    case .touch: "触屏手柄"
    case .sticks: "摇杆调节"
    case .mapping: "按键与轴映射"
    case .vibration: "震动"
    case .menu: "菜单操作"
    }
  }
  var icon: AppIcon {
    switch self {
    case .connection: .controller
    case .touch: .touch
    case .sticks, .mapping: .sliders
    case .vibration: .vibration
    case .menu: .touch
    }
  }
  var subtitle: String {
    switch self {
    case .connection: "状态 · 按键 · 摇杆"
    case .touch: "布局 · 大小 · 透明度"
    case .sticks: "死区与方向"
    case .mapping: "按键 · 组合 · 轴"
    case .vibration: "强度与震动通道"
    case .menu: "触屏与手柄快捷操作"
    }
  }
}

struct ControllerView: View {
  @EnvironmentObject private var streaming: StreamingFeatureStore
  @State private var selection: ControllerRoute?
  var isActive = true

  var body: some View {
    AdaptiveSectionNavigation(title: "控制器", selection: $selection) {
      ControllerOverview(input: streaming.input, isActive: isActive)
    } detail: { route in
      switch route {
      case .connection: ControllerTestView(input: streaming.input, isActive: isActive)
      case .touch: ControllerTouchSettingsView()
      case .sticks: ControllerStickSettingsView()
      case .mapping: ControllerMappingSettingsView()
      case .vibration: ControllerVibrationView(isActive: isActive, input: streaming.input)
      case .menu: ControllerMenuSettingsView()
      }
    }
  }
}

private struct ControllerOverview: View {
  @ObservedObject var input: StreamInputSession
  let isActive: Bool
  @Environment(\.scenePhase) private var scenePhase
  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      Text(LocalizedStringKey(input.controllerName ?? "控制器"))
        .appFont(.section, weight: .semibold)
      Group {
        if input.hasController {
          Text("已连接 \(input.controllerCount) 个手柄")
        } else {
          Text("连接蓝牙手柄，或使用串流内的触屏手柄。")
        }
      }
      .appFont(.caption, weight: .regular).foregroundStyle(AppThemePalette.textSecondary)
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .task(id: isActive && scenePhase == .active) {
      guard isActive, scenePhase == .active else { return }
      while !Task.isCancelled {
        input.refreshDeviceStatus()
        try? await Task.sleep(for: .milliseconds(500))
      }
    }
  }
}

struct ControllerTestView: View {
  @ObservedObject var input: StreamInputSession
  let isActive: Bool
  @Environment(\.scenePhase) private var scenePhase
  @State private var locked = false
  @State private var previousA = false
  @State private var exitStarted: Date?
  @State private var readingFrequency = 0
  @State private var history = MobileControllerInputHistory()
  var body: some View {
    AppSettingsForm {
      Section("连接") {
        MobileControllerDevicePicker(input: input)
        VStack(alignment: .leading, spacing: 12) {
          HStack(alignment: .firstTextBaseline) {
            Text(
              LocalizedStringKey(input.controllerName ?? (input.hasController ? "游戏手柄" : "等待连接"))
            )
            .appFont(.body, weight: .semibold)
            Spacer(minLength: 8)
            if let battery = input.controllerBatteryPercent {
              Text("\(battery)%").appFont(.caption)
                .foregroundStyle(AppThemePalette.textSecondary)
            }
          }
          if input.hasController {
            Label(locked ? "测试中" : "已连接", systemImage: "circle.fill")
              .appFont(.caption).foregroundStyle(AppThemePalette.brand)
            DisclosureGroup("设备详情") {
              AppFactGroup(facts: deviceFacts, usesSurface: false)
                .padding(.top, 8)
            }
            .appFont(.body).tint(AppThemePalette.brand)
            .frame(minHeight: 44).accessibilityIdentifier("controller-device-details")
          } else {
            Text("在系统设置的蓝牙页面配对 Xbox、PlayStation 或兼容的游戏手柄。")
              .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
          }
        }
        .padding(16).appSurface(radius: AppLayoutPolicy.cardRadius)
        .accessibilityIdentifier("controller-connection-summary")
        .appSettingsInformation(ownsPadding: true)
      }

      Section("手柄测试") {
        MobileControllerPicture(
          highlighted: MobileShortcutButton.allCases.filter {
            $0.isPressed(in: input.liveState)
          })
        ControllerInputHistoryStrip(entries: history.entries).appSettingsInformation()
        AppSettingsActionButton(locked ? "结束测试" : "A 开始测试") { setLocked(!locked) }
        Text(
          LocalizedStringKey(
            locked ? "测试期间按键显示在图片上。长按菜单 + 视图 1 秒退出。" : "按 A 锁定测试，实体输入在测试期间由此页面接管。")
        )
        .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
        .appSettingsInformation()
      }
      Section {
        DisclosureGroup("摇杆与扳机") {
          AppFactGroup(facts: [
            ("左摇杆 X", number(input.liveState.leftThumbX)),
            ("左摇杆 Y", number(input.liveState.leftThumbY)),
            ("右摇杆 X", number(input.liveState.rightThumbX)),
            ("右摇杆 Y", number(input.liveState.rightThumbY)),
            ("LT", number(input.liveState.leftTrigger)),
            ("RT", number(input.liveState.rightTrigger)),
          ])
        }.frame(minHeight: 44).appSettingsInformation()
      }
    }
    .scrollContentBackground(.hidden).appThemeCanvas()
    .task(id: isActive && scenePhase == .active) {
      guard isActive && scenePhase == .active else {
        setLocked(false)
        return
      }
      previousA = input.rawControllerState.buttonMask & 16 != 0
      var samples = 0
      var sampleStarted = Date()
      while !Task.isCancelled {
        input.refreshDeviceStatus()
        history.sample(input.liveState)
        samples += 1
        let elapsed = Date().timeIntervalSince(sampleStarted)
        if elapsed >= 1 {
          readingFrequency = Int((Double(samples) / elapsed).rounded())
          samples = 0
          sampleStarted = Date()
        }
        let raw = input.rawControllerState
        let a = MobileShortcutButton.a.isPressed(in: raw)
        if !locked, a && !previousA { setLocked(true) }
        if locked, MobileShortcutButton.menu.isPressed(in: raw),
          MobileShortcutButton.view.isPressed(in: raw)
        {
          if exitStarted == nil { exitStarted = Date() }
          if Date().timeIntervalSince(exitStarted ?? Date()) >= 1 { setLocked(false) }
        } else {
          exitStarted = nil
        }
        if !input.hasController { setLocked(false) }
        previousA = a
        try? await Task.sleep(for: .milliseconds(16))
      }
      setLocked(false)
      readingFrequency = 0
    }
    .onDisappear { setLocked(false) }
    .onChange(of: input.selectedControllerID) { _, _ in history.reset() }
    .onChange(of: input.hasController) { _, _ in history.reset() }
  }
  private var deviceFacts: [(String, String)] {
    var facts: [(String, String)] = [("按键布局", "标准 Xbox 按键")]
    if input.controllerCount > 1 { facts.append(("连接数量", "\(input.controllerCount)")) }
    if let category = input.controllerCategory { facts.append(("设备类型", category)) }
    if let haptics = input.controllerSupportsHaptics {
      facts.append(("震动支持", haptics ? "支持" : "暂无支持"))
    }
    if let attached = input.controllerIsAttachedToDevice {
      facts.append(("连接形式", attached ? "设备附加控制器" : "外接控制器"))
    }
    if locked { facts.append(("测试采样", "\(readingFrequency) Hz")) }
    return facts
  }

  private func setLocked(_ value: Bool) {
    locked = value
    exitStarted = nil
    input.setSuppressed(value, reason: .settingsTest)
  }
  private func number(_ value: Float) -> String {
    value.formatted(.number.precision(.fractionLength(2)))
  }
}

/// 输入记录沿原有十条历史消费，横向内容按系统字号自然增高。
struct ControllerInputHistoryStrip: View {
  @Environment(\.locale) private var locale
  let entries: [MobileShortcutButton]

  var body: some View {
    ScrollViewReader { proxy in
      ScrollView(.horizontal) {
        HStack(spacing: 16) {
          ForEach(Array(entries.enumerated()), id: \.offset) { index, button in
            Text(LocalizedStringKey(button.title))
              .appFont(.body, weight: .semibold)
              .foregroundStyle(
                index == entries.count - 1
                  ? AppThemePalette.brand : AppThemePalette.textSecondary
              )
              .frame(minWidth: 24, minHeight: 44).id(index)
          }
        }
      }
      .scrollIndicators(.hidden)
      .fixedSize(horizontal: false, vertical: true).frame(minHeight: 44)
      .overlay(alignment: .bottom) {
        Rectangle().fill(AppThemePalette.icon.opacity(0.4)).frame(height: 1)
          .accessibilityHidden(true)
      }
      .onChange(of: entries) { _, values in
        if !values.isEmpty { proxy.scrollTo(values.count - 1, anchor: .trailing) }
      }
    }
    .frame(minHeight: 44).contentShape(Rectangle())
    .accessibilityElement(children: .ignore)
    .accessibilityLabel("按键记录")
    .accessibilityValue(historyValue)
    .accessibilityIdentifier("controller-input-history")
  }

  private var historyValue: String {
    guard !entries.isEmpty else { return appLocalizedString("等待输入", locale: locale) }
    let formatter = ListFormatter()
    formatter.locale = locale
    let names = entries.map { appLocalizedString($0.title, locale: locale) }
    return formatter.string(from: names) ?? names.joined(separator: "、")
  }
}

struct ControllerTouchSettingsView: View {
  @EnvironmentObject private var settings: AppSettingsStore
  @State private var presentation: TouchWorkspacePresentation?
  var body: some View {
    AppSettingsForm {
      Section {
        AppSettingsPicker(
          "触屏手柄", selection: $settings.mobile.touchControllerMode,
          selectedTitle: settings.mobile.touchControllerMode.title
        ) {
          ForEach(TouchControllerMode.allCases) {
            Text(LocalizedStringKey($0.title)).tag($0)
          }
        }
        .appSettingsHint("自动模式在实体手柄断开时显示", identifier: "controller-touch")
        AppSettingsPicker(
          "布局模板", selection: $settings.mobile.touchLayoutID,
          selectedTitle: settings.mobile.selectedTouchLayoutName
        ) {
          Text("通用 Xbox").tag(TouchLayoutTemplate.standardID)
          ForEach(settings.mobile.touchLayoutTemplates) { Text(verbatim: $0.name).tag($0.id) }
        }
        .accessibilityIdentifier("controller-touch-layout")
      } header: {
        Text("触屏")
      }
      Section("布局与外观") {
        AppSettingsActionButton("自定义布局") { present(.editor) }
          .accessibilityIdentifier("controller-touch-customize")
        AppSettingsActionButton("触屏手柄调整") { present(.adjustment) }
          .accessibilityIdentifier("controller-touch-adjust")
        NavigationLink {
          SavedTouchLayoutsView()
        } label: {
          Text("已保存的游戏布局")
        }
        .frame(minHeight: 44).accessibilityIdentifier("controller-saved-touch-layouts")
      }
    }
    .appThemeCanvas()
    .fullScreenCover(
      item: $presentation,
      onDismiss: {
        AppOrientationCoordinator.shared.refresh()
      },
      content: { item in
        NavigationStack {
          switch item.kind {
          case .editor:
            TouchLayoutEditorView(template: item.template, appearance: item.appearance)
          case .adjustment:
            TouchLayoutAdjustmentView(template: item.template, appearance: item.appearance)
          }
        }
        .environment(\.appFloatingNavigationInset, 0)
        .environment(\.appCanvasBounds, nil)
        .preferredColorScheme(.dark)
      }
    )
  }

  private func present(_ kind: TouchWorkspacePresentation.Kind) {
    presentation = TouchWorkspacePresentation(
      kind: kind, template: settings.mobile.selectedTouchTemplate,
      appearance: settings.mobile.selectedTouchAppearance)
  }
}

private struct TouchWorkspacePresentation: Identifiable {
  enum Kind { case editor, adjustment }
  let id = UUID()
  let kind: Kind
  let template: TouchLayoutTemplate?
  let appearance: TouchLayoutAppearance
}

struct ControllerStickSettingsView: View {
  @EnvironmentObject private var settings: AppSettingsStore
  @State private var selectedStick = "leftX"
  var body: some View {
    AppSettingsForm {
      Section("摇杆调节") {
        MobileControllerPicture(
          selected: selectedStick, mode: "sticks", select: { selectedStick = $0 })
        Label("点按摇杆选择调节部位", systemImage: "hand.tap")
          .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
          .appSettingsInformation()
        if selectedStick == "leftX" {
          preferenceSlider(
            "左摇杆死区", value: $settings.mobile.leftStickDeadzone, range: 0...0.3)
          AppSettingsToggle("反转左摇杆 Y 轴", isOn: $settings.mobile.invertLeftY)
        } else {
          preferenceSlider(
            "右摇杆死区", value: $settings.mobile.rightStickDeadzone, range: 0...0.3)
          AppSettingsToggle("反转右摇杆 Y 轴", isOn: $settings.mobile.invertRightY)
        }
      }
    }
    .appThemeCanvas()
  }
}

struct ControllerMappingSettingsView: View {
  @EnvironmentObject private var streaming: StreamingFeatureStore
  var body: some View {
    AppSettingsForm {
      Section("按键与轴映射") {
        MobileControllerDevicePicker(input: streaming.input)
        MobileGamepadMappingControls()
      }
    }
    .scrollContentBackground(.hidden).appThemeCanvas()
  }
}

struct ControllerVibrationView: View {
  let isActive: Bool
  @EnvironmentObject private var settings: AppSettingsStore
  @ObservedObject var input: StreamInputSession
  @State private var engine: CHHapticEngine?
  @State private var player: (any CHHapticPatternPlayer)?
  @State private var testTask: Task<Void, Never>?
  @State private var message: String?
  @State private var selectedChannel = "leftHandle"
  var body: some View {
    AppSettingsForm {
      Section {
        AppSettingsToggle("游戏震动", isOn: $settings.mobile.vibrationEnabled)
          .appSettingsHint("按设备支持的通道输出，立即生效", identifier: "controller-vibration")
        AppSettingsPicker(
          "震动预设",
          selection: Binding(
            get: {
              settings.mobile.vibrationChannels.uniformStrength?.rawValue ?? "custom"
            },
            set: { raw in
              if let strength = MobileVibrationStrength(rawValue: raw) {
                settings.mobile.vibrationChannels = MobileVibrationChannels(
                  strength)
              }
            }),
          selectedTitle: settings.mobile.vibrationChannels.uniformStrength?.title ?? "自定义"
        ) {
          if settings.mobile.vibrationChannels.uniformStrength == nil {
            Text("自定义").tag("custom")
          }
          ForEach(MobileVibrationStrength.allCases) {
            Text(LocalizedStringKey($0.title)).tag($0.rawValue)
          }
        }
        preferenceSlider("震动强度", value: $settings.mobile.vibrationIntensity, range: 0...1)
      }
      Section("通道调节") {
        MobileControllerPicture(
          selected: selectedChannel, mode: "vibration", select: { selectedChannel = $0 })
        Label("点按握把或扳机选择通道", systemImage: "hand.tap").appFont(.caption).foregroundStyle(
          AppThemePalette.textSecondary
        )
        .appSettingsInformation()
        vibrationPicker(channelName, selection: channelBinding)
        AppSettingsActionButton(channelTitle(selectedLocality)) { test(selectedLocality) }
          .disabled(
            !settings.mobile.vibrationEnabled
              || input.currentController?.haptics?.supportedLocalities.contains(
                selectedLocality) != true
          )
        if input.currentController?.haptics == nil {
          Text("连接支持触觉的游戏手柄后可测试。").appFont(.caption).foregroundStyle(
            AppThemePalette.textSecondary
          )
          .appSettingsInformation()
        }
        if let message {
          Text(LocalizedStringKey(message)).appFont(.caption).foregroundStyle(
            AppThemePalette.textSecondary
          )
          .appSettingsInformation()
        }
      }
    }
    .scrollContentBackground(.hidden).appThemeCanvas()
    .onDisappear { stop() }
    .onChange(of: isActive) { _, active in
      if !active { stop() }
    }
    .onChange(of: settings.mobile.vibrationEnabled) { _, _ in stop() }
    .onChange(of: settings.mobile.vibrationChannels) { _, _ in stop() }
    .onChange(of: settings.mobile.vibrationIntensity) { _, _ in stop() }
    .onReceive(NotificationCenter.default.publisher(for: .GCControllerDidDisconnect)) { _ in
      stop()
    }
    .onReceive(
      NotificationCenter.default.publisher(for: UIApplication.willResignActiveNotification)
    ) { _ in stop()
    }
  }
  private var selectedLocality: GCHapticsLocality {
    switch selectedChannel {
    case "leftTrigger": .leftTrigger
    case "rightTrigger": .rightTrigger
    case "rightHandle": .rightHandle
    default: .leftHandle
    }
  }
  private var channelName: String {
    switch selectedChannel {
    case "leftTrigger": "左扳机"
    case "rightTrigger": "右扳机"
    case "rightHandle": "右握把"
    default: "左握把"
    }
  }
  private var channelBinding: Binding<MobileVibrationStrength> {
    switch selectedChannel {
    case "leftTrigger": $settings.mobile.vibrationChannels.leftTrigger
    case "rightTrigger": $settings.mobile.vibrationChannels.rightTrigger
    case "rightHandle": $settings.mobile.vibrationChannels.rightHandle
    default: $settings.mobile.vibrationChannels.leftHandle
    }
  }
  private func channelTitle(_ locality: GCHapticsLocality) -> String {
    switch locality {
    case .leftHandle: "测试左握把"
    case .rightHandle: "测试右握把"
    case .leftTrigger: "测试左扳机"
    default: "测试右扳机"
    }
  }
  private func vibrationPicker(_ title: String, selection: Binding<MobileVibrationStrength>)
    -> some View
  {
    AppSettingsPicker(title, selection: selection, selectedTitle: selection.wrappedValue.title) {
      ForEach(MobileVibrationStrength.allCases) { Text(LocalizedStringKey($0.title)).tag($0) }
    }
  }
  private func test(_ locality: GCHapticsLocality) {
    stop()
    guard let haptics = input.currentController?.haptics,
      let engine = haptics.createEngine(withLocality: locality)
    else { return }
    do {
      self.engine = engine
      try engine.start()
      let pattern = try CHHapticPattern(
        events: [
          CHHapticEvent(
            eventType: .hapticContinuous,
            parameters: [
              CHHapticEventParameter(
                parameterID: .hapticIntensity,
                value: testMagnitude(locality))
            ], relativeTime: 0, duration: 0.2)
        ], parameters: [])
      let player = try engine.makePlayer(with: pattern)
      self.player = player
      try player.start(atTime: 0)
      testTask = Task {
        try? await Task.sleep(for: .milliseconds(250))
        guard !Task.isCancelled else { return }
        stop()
      }
    } catch {
      message = "震动测试失败，请重新连接手柄"
      stop()
    }
  }
  private func testMagnitude(_ locality: GCHapticsLocality) -> Float {
    let channels = settings.mobile.vibrationChannels
    let strength: MobileVibrationStrength
    switch locality {
    case .leftHandle: strength = channels.leftHandle
    case .rightHandle: strength = channels.rightHandle
    case .leftTrigger: strength = channels.leftTrigger
    default: strength = channels.rightTrigger
    }
    return strength.magnitude(1, trigger: locality == .leftTrigger || locality == .rightTrigger)
      * Float(settings.mobile.vibrationIntensity)
  }
  private func stop() {
    testTask?.cancel()
    testTask = nil
    try? player?.stop(atTime: 0)
    player = nil
    engine?.stop(completionHandler: nil)
    engine = nil
  }
}

struct ControllerMenuSettingsView: View {
  @EnvironmentObject private var settings: AppSettingsStore
  var body: some View {
    AppSettingsForm {
      Section("触屏") {
        VStack(alignment: .leading, spacing: 12) {
          Text("打开串流菜单").appFont(.body, weight: .semibold)
          AppWrappingLayout(spacing: 24, lineSpacing: 8) {
            Label("轻点画面", systemImage: "hand.tap")
            Label("双指内滑", systemImage: "arrow.down.right.and.arrow.up.left")
          }
          .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
        }
        .padding(AppLayoutPolicy.rowInset)
        .frame(maxWidth: .infinity, alignment: .leading)
        .appSurface()
        .appSettingsInformation(ownsPadding: true)
        .accessibilityIdentifier("controller-menu-touch-group")
      }
      Section("手柄") {
        MobileShortcutControls()
      }
    }
    .appThemeCanvas()
  }
}

private func preferenceSlider(
  _ title: String, value: Binding<Double>, range: ClosedRange<Double>
)
  -> some View
{
  let display = Text("\(Int(value.wrappedValue * 100))%")
  return VStack(alignment: .leading, spacing: 8) {
    LabeledContent {
      display
    } label: {
      Text(LocalizedStringKey(title))
    }
    Slider(value: value, in: range, step: 0.01)
      .frame(minHeight: 44)
      .accessibilityLabel(Text(LocalizedStringKey(title)))
      .accessibilityValue(display)
  }
  .padding(.vertical, 4)
}
