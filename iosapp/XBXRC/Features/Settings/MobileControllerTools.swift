import Combine
import CoreHaptics
import GameController
import SwiftUI

enum MobileControlAxis: String, Codable, CaseIterable, Sendable {
    case leftX, leftY, rightX, rightY
    var title: String {
        switch self {
        case .leftX: "左摇杆 X"
        case .leftY: "左摇杆 Y"
        case .rightX: "右摇杆 X"
        case .rightY: "右摇杆 Y"
        }
    }
    func value(in state: IOSStreamGamepadState) -> Float {
        switch self {
        case .leftX: state.leftThumbX
        case .leftY: state.leftThumbY
        case .rightX: state.rightThumbX
        case .rightY: state.rightThumbY
        }
    }
    func write(_ value: Float, to state: inout IOSStreamGamepadState) {
        switch self {
        case .leftX: state.leftThumbX = value
        case .leftY: state.leftThumbY = value
        case .rightX: state.rightThumbX = value
        case .rightY: state.rightThumbY = value
        }
    }
}

enum MobileControlBinding: Codable, Equatable, Sendable {
    case buttons([MobileShortcutButton])
    case axis(MobileControlAxis, inverted: Bool)
    case rawButtons([String], any: Bool)
    case rawAxis(String, inverted: Bool, fullRange: Bool)

    var buttonSources: [String]? {
        switch self {
        case .buttons(let keys): keys.map(\.rawValue)
        case .rawButtons(let keys, _): keys
        default: nil
        }
    }
    var axisSource: String? {
        switch self {
        case .axis(let axis, _): axis.rawValue
        case .rawAxis(let key, _, _): key
        default: nil
        }
    }
    func title(locale: Locale) -> String {
        switch self {
        case .buttons(let buttons):
            buttons.map { appLocalizedString($0.title, locale: locale) }.joined(separator: " + ")
        case .axis(let axis, let inverted):
            "\(appLocalizedString(axis.title, locale: locale)) \(inverted ? "−" : "+")"
        case .rawButtons(let keys, let any):
            keys.map {
                MobileShortcutButton(rawValue: $0).map {
                    appLocalizedString($0.title, locale: locale)
                } ?? $0.replacingOccurrences(of: "extra:", with: "")
            }.joined(separator: any ? " / " : " + ")
        case .rawAxis(let key, let inverted, let full):
            "\(key.replacingOccurrences(of: "axis:", with: "")) \(inverted ? "−" : "+")\(full ? " " + appLocalizedString("全范围", locale: locale) : "")"
        }
    }
    static func normalized(_ profiles: [String: [String: Self]]) -> [String: [String: Self]] {
        let targets = Set(
            MobileShortcutButton.allCases.map(\.rawValue)
                + MobileControlAxis.allCases.map(\.rawValue))
        var result: [String: [String: Self]] = [:]
        for key in profiles.keys.sorted().prefix(64) where !key.isEmpty && key.utf8.count <= 256 {
            var bindings: [String: Self] = [:]
            var chords: Set<String> = []
            for target in (profiles[key] ?? [:]).keys.sorted() where targets.contains(target) {
                guard let binding = profiles[key]?[target] else { continue }
                if case .buttons(let buttons) = binding {
                    guard MobileShortcutButton(rawValue: target) != nil,
                        (1...4).contains(buttons.count), Set(buttons).count == buttons.count
                    else { continue }
                    let normalized = MobileShortcutButton.normalized(buttons)
                    let chord = normalized.map(\.rawValue).sorted().joined(separator: ",")
                    if normalized.count > 1 && !chords.insert(chord).inserted { continue }
                    bindings[target] = .buttons(normalized)
                } else if case .rawButtons(let keys, let any) = binding {
                    guard MobileShortcutButton(rawValue: target) != nil,
                        (1...(any ? 128 : 4)).contains(keys.count), Set(keys).count == keys.count,
                        keys.allSatisfy({ !$0.isEmpty && $0.utf8.count <= 256 })
                    else { continue }
                    let chord = keys.sorted().joined(separator: ",")
                    if !any && keys.count > 1 && !chords.insert(chord).inserted { continue }
                    bindings[target] = .rawButtons(keys.sorted(), any: any)
                } else if case .rawAxis(let key, let inverted, let full) = binding {
                    guard !key.isEmpty && key.utf8.count <= 256 else { continue }
                    bindings[target] = .rawAxis(
                        key, inverted: inverted,
                        fullRange: MobileControlAxis(rawValue: target) == nil && full)
                } else {
                    bindings[target] = binding
                }
            }
            if !bindings.isEmpty { result[key] = bindings }
        }
        return result
    }

    static func apply(
        _ raw: IOSStreamGamepadState, bindings: [String: Self],
        buttons: [String: Float] = [:], axes: [String: Float] = [:]
    ) -> IOSStreamGamepadState {
        let samples = Dictionary(
            uniqueKeysWithValues: MobileShortcutButton.allCases.map {
                ($0.rawValue, $0.value(in: raw))
            }
        )
        .merging(buttons) { _, new in new }
        let axisSamples = Dictionary(
            uniqueKeysWithValues: MobileControlAxis.allCases.map {
                ($0.rawValue, $0.value(in: raw))
            }
        )
        .merging(axes) { _, new in new }
        let active = bindings.filter { entry in
            guard let keys = entry.value.buttonSources, keys.count > 1 else { return false }
            if case .rawButtons(_, let any) = entry.value, any { return false }
            return keys.allSatisfy { (samples[$0] ?? 0) > 0.5 }
        }
        let consumed = Set(active.values.flatMap { $0.buttonSources ?? [] })
        var output = IOSStreamGamepadState.neutral
        for target in MobileShortcutButton.allCases.map(\.rawValue)
            + MobileControlAxis.allCases.map(\.rawValue)
        {
            let isAxis = MobileControlAxis(rawValue: target) != nil
            let binding =
                bindings[target]
                ?? (isAxis
                    ? .axis(MobileControlAxis(rawValue: target)!, inverted: false)
                    : .buttons([MobileShortcutButton(rawValue: target)!]))
            let value: Float
            switch binding {
            case .buttons(let keys):
                value =
                    keys.count > 1
                    ? (active[target] != nil ? 1 : 0)
                    : keys.first.map {
                        consumed.contains($0.rawValue) ? 0 : samples[$0.rawValue] ?? 0
                    } ?? 0
            case .rawButtons(let keys, let any):
                value =
                    !any && keys.count > 1
                    ? (active[target] != nil ? 1 : 0)
                    : keys.map { consumed.contains($0) ? 0 : samples[$0] ?? 0 }.max() ?? 0
            case .axis(let source, let inverted):
                let v = source.value(in: raw) * (inverted ? -1 : 1)
                value = isAxis ? v : max(0, v)
            case .rawAxis(let key, let inverted, let full):
                let rest: Float = full ? (inverted ? 1 : -1) : 0
                let sample = axisSamples[key] ?? rest
                let v = (sample.isFinite ? min(1, max(-1, sample)) : rest) * (inverted ? -1 : 1)
                value = isAxis ? v : full ? (v + 1) / 2 : max(0, v)
            }
            if let axis = MobileControlAxis(rawValue: target) {
                axis.write(value, to: &output)
            } else {
                MobileShortcutButton(rawValue: target)?.write(value, to: &output)
            }
        }
        return output
    }

    static func assign(_ binding: Self, target: String, to bindings: inout [String: Self]) {
        if let captured = binding.buttonSources, captured.count == 1,
            let previous = MobileShortcutButton(rawValue: target)
        {
            let old = bindings[target] ?? .buttons([previous])
            if old.buttonSources?.count == 1 {
                for other in MobileShortcutButton.allCases where other.rawValue != target {
                    if (bindings[other.rawValue] ?? .buttons([other])).buttonSources == captured {
                        bindings[other.rawValue] = old
                        break
                    }
                }
            }
        }
        if let captured = binding.axisSource, let previous = MobileControlAxis(rawValue: target) {
            let old = bindings[target] ?? .axis(previous, inverted: false)
            if old.axisSource != nil {
                for other in MobileControlAxis.allCases where other.rawValue != target {
                    if (bindings[other.rawValue] ?? .axis(other, inverted: false)).axisSource
                        == captured
                    {
                        bindings[other.rawValue] = old
                        break
                    }
                }
            }
        }
        bindings[target] = binding
    }
}

/// 原始设备来源保留系统元素键，附加按钮和非居中轴沿同一录入与映射合同。
struct MobileRawPhysicalCapture {
    var ready = false
    var peak: [String] = []
    var axisPeak: (String, Float)?
    var rests: [String: Float] = [:]
    var includesAxes = false
    var axisOnly = false

    mutating func sample(buttons: [String: Float], axes: [String: Float]) -> MobileControlBinding? {
        let held = buttons.keys.filter { (buttons[$0] ?? 0) > 0.5 }.sorted()
        let centered = axes.allSatisfy { abs($0.value - (rests[$0.key] ?? 0)) <= 0.15 }
        guard ready else {
            ready = held.isEmpty && (!includesAxes || centered)
            return nil
        }
        if !axisOnly && held.count > peak.count { peak = held }
        if includesAxes {
            for (key, value) in axes.sorted(by: { $0.key < $1.key }) {
                let delta = value - (rests[key] ?? 0)
                if abs(delta) > max(0.45, abs(axisPeak?.1 ?? 0)) { axisPeak = (key, delta) }
            }
        }
        guard held.isEmpty else { return nil }
        if !peak.isEmpty {
            let standard = peak.compactMap(MobileShortcutButton.init(rawValue:))
            return standard.count == peak.count ? .buttons(standard) : .rawButtons(peak, any: false)
        }
        if centered, let (key, delta) = axisPeak {
            if let axis = MobileControlAxis(rawValue: key) {
                return .axis(axis, inverted: delta < 0)
            }
            return .rawAxis(
                key, inverted: delta < 0, fullRange: !axisOnly && abs(rests[key] ?? 0) > 0.8)
        }
        return nil
    }
}

enum MobileCaptureTarget: Equatable {
    case shortcut(MobileShortcutTarget)
    case control(String, positive: Bool = true)
}

/// 按下沿进入历史；持续按住保留一条，重复按压可再次记录。
struct MobileControllerInputHistory {
    private(set) var entries: [MobileShortcutButton] = []
    private var held: Set<MobileShortcutButton> = []

    mutating func sample(_ state: IOSStreamGamepadState) {
        let pressed = MobileShortcutButton.allCases.filter { $0.isPressed(in: state) }
        entries.append(contentsOf: pressed.filter { !held.contains($0) })
        entries = Array(entries.suffix(10))
        held = Set(pressed)
    }

    mutating func reset() { self = Self() }
}

@MainActor
final class MobileControllerCaptureSession: ObservableObject {
    @Published private(set) var target: MobileCaptureTarget?
    @Published private(set) var pressed: [MobileShortcutButton] = []
    @Published private(set) var message: String?
    private var reader = MobileRawPhysicalCapture()
    private weak var controller: GCController?
    private weak var input: StreamInputSession?
    private var profile = ""

    func begin(_ target: MobileCaptureTarget, input: StreamInputSession) {
        let snapshot = input.controllerSnapshot()
        guard let controller = snapshot.controller else {
            message = "连接手柄后选择录入目标。"
            return
        }
        cancel()
        self.controller = controller
        self.input = input
        self.profile = snapshot.layoutKey
        self.target = target
        message = "松开当前按键并回中摇杆，再输入按键、组合或方向；释放后自动保存。"
        let axisOnly: Bool = {
            if case .control(let key, _) = target { return MobileControlAxis(rawValue: key) != nil }
            return false
        }()
        reader = MobileRawPhysicalCapture(
            rests: Dictionary(
                uniqueKeysWithValues: snapshot.axes.map { key, value in
                    (key, MobileControlAxis(rawValue: key) == nil && abs(value) > 0.8 ? value : 0)
                }),
            includesAxes: {
                if case .control = target { return true }
                return false
            }(), axisOnly: axisOnly)
        input.setSuppressed(true, reason: .settingsCapture)
    }
    func poll(input: StreamInputSession, settings: AppSettingsStore) {
        guard let target else { return }
        let snapshot = input.controllerSnapshot()
        guard let controller, let current = snapshot.controller, controller === current else {
            cancel()
            message = "手柄连接已改变，录入已结束。"
            return
        }
        var preferences = settings.mobile
        let raw: IOSStreamGamepadState
        let buttons: [String: Float]
        let axes: [String: Float]
        if case .shortcut = target {
            // 快捷操作显示并保存标准 Xbox 目标，和串流内映射后的判定保持一致。
            raw = snapshot.mapped(using: preferences)
            buttons = Dictionary(
                uniqueKeysWithValues: MobileShortcutButton.allCases.map {
                    ($0.rawValue, $0.value(in: raw))
                })
            axes = [:]
        } else {
            raw = snapshot.raw
            buttons = snapshot.buttons
            axes = snapshot.axes
        }
        pressed = MobileShortcutButton.allCases.filter { $0.isPressed(in: raw) }
        guard var binding = reader.sample(buttons: buttons, axes: axes) else { return }
        switch target {
        case .shortcut(let shortcut):
            guard case .buttons(let buttons) = binding, buttons.count <= 4 else {
                cancel()
                message = "组合最多 4 个按键，请重新录入。"
                return
            }
            guard !preferences.shortcutConflicts(buttons, for: shortcut) else {
                cancel()
                message = "两个菜单需要使用不同的组合。"
                return
            }
            preferences.setShortcut(buttons, for: shortcut)
        case .control(let control, let positive):
            if !positive {
                switch binding {
                case .axis(let axis, let inverted): binding = .axis(axis, inverted: !inverted)
                case .rawAxis(let key, let inverted, let full):
                    binding = .rawAxis(key, inverted: !inverted, fullRange: full)
                default: break
                }
            }
            if let buttons = binding.buttonSources,
                buttons.count > 4 || MobileShortcutButton(rawValue: control) == nil
            {
                cancel()
                message = "该目标需要录入对应摇杆轴。"
                return
            }
            var bindings = preferences.gamepadBindings[profile] ?? [:]
            if let buttons = binding.buttonSources, buttons.count > 1,
                bindings.contains(where: {
                    $0.key != control && $0.value.buttonSources?.sorted() == buttons.sorted()
                })
            {
                cancel()
                message = "该组合已分配给其他按键，请重新录入。"
                return
            }
            if preferences.gamepadBindings[profile] == nil
                && preferences.gamepadBindings.count >= 64
            {
                cancel()
                message = "已达到 64 个设备配置上限。"
                return
            }
            MobileControlBinding.assign(binding, target: control, to: &bindings)
            preferences.gamepadBindings[profile] = bindings
        }
        settings.mobile = preferences
        cancel()
        message = "已保存，录入完成。"
    }
    func cancel() {
        input?.setSuppressed(false, reason: .settingsCapture)
        input = nil
        controller = nil
        target = nil
        pressed = []
        message = nil
        reader = MobileRawPhysicalCapture()
    }
}

struct MobileControllerPicture: View {
    var highlighted: [MobileShortcutButton] = []
    var selected: String?
    var mode: String = "mapping"
    var select: ((String) -> Void)?

    private var targets: [(String, CGFloat, CGFloat)] {
        if mode == "vibration" {
            return [
                ("leftTrigger", 104, 6), ("rightTrigger", 268, 6),
                ("leftHandle", 76, 180), ("rightHandle", 298, 180),
            ]
        }
        if mode == "sticks" { return [("leftX", 90, 74), ("rightX", 233, 131)] }
        return [
            ("home", 186, 35), ("leftTrigger", 104, 6), ("rightTrigger", 268, 6),
            ("a", 282, 98), ("b", 307, 74), ("x", 257, 73), ("y", 281, 49),
            ("leftShoulder", 83, 16), ("rightShoulder", 289, 16),
            ("view", 159, 72), ("menu", 213, 72), ("leftStick", 90, 74), ("rightStick", 233, 131),
            ("up", 137, 115), ("down", 137, 151), ("left", 117, 133), ("right", 157, 133),
        ]
    }
    var body: some View {
        VStack(spacing: 12) {
            GeometryReader { geometry in
                let scale = geometry.size.width / 372
                ZStack(alignment: .topLeading) {
                    Image("ControllerSeries").resizable().scaledToFit()
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                    ForEach(targets, id: \.0) { target in
                        let active =
                            selected == target.0 || highlighted.contains { $0.rawValue == target.0 }
                        if mode == "mapping" {
                            Circle().fill(active ? AppThemePalette.brand.opacity(0.65) : .clear)
                                .overlay(
                                    Circle().stroke(AppThemePalette.brand.opacity(active ? 1 : 0.3))
                                )
                                .frame(width: 18, height: 18)
                                .position(x: target.1 * scale, y: max(22, target.2 * scale))
                                .allowsHitTesting(false)
                        } else if let select {
                            Button {
                                select(target.0)
                            } label: {
                                Circle().fill(AppThemePalette.brand.opacity(active ? 0.65 : 0.08))
                                    .overlay(
                                        Circle().stroke(
                                            AppThemePalette.brand.opacity(active ? 1 : 0.3))
                                    )
                                    .frame(width: 44, height: 44).contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .frame(width: 44, height: 44)
                            .offset(x: target.1 * scale - 22, y: max(22, target.2 * scale) - 22)
                            .accessibilityLabel(Text(LocalizedStringKey(label(target.0))))
                            .accessibilityIdentifier("controller-hotspot-\(mode)-\(target.0)")
                            .accessibilityAddTraits(active ? .isSelected : [])
                        }
                    }
                }
                .frame(
                    width: geometry.size.width, height: geometry.size.height, alignment: .topLeading
                )
            }
            .aspectRatio(372.0 / 258.0, contentMode: .fit)
            .frame(maxWidth: AppLayoutPolicy.controllerArtworkMaximumWidth)
            .overlay {
                if mode == "mapping", let select {
                    GeometryReader { geometry in
                        Color.clear.contentShape(Rectangle())
                            .gesture(
                                // 点按选择热点，滚动手势交给页面容器。
                                SpatialTapGesture().onEnded { gesture in
                                    let scale = geometry.size.width / 372
                                    if let nearest = targets.min(by: {
                                        hypot(
                                            $0.1 * scale - gesture.location.x,
                                            max(22, $0.2 * scale) - gesture.location.y)
                                            < hypot(
                                                $1.1 * scale - gesture.location.x,
                                                max(22, $1.2 * scale) - gesture.location.y)
                                    }),
                                        hypot(
                                            nearest.1 * scale - gesture.location.x,
                                            max(22, nearest.2 * scale) - gesture.location.y) <= 30
                                    {
                                        select(nearest.0)
                                    }
                                }
                            )
                            .accessibilityHidden(true)
                    }
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("controller-picture-\(mode)")
        }
        .frame(maxWidth: .infinity)
        .appSettingsInformation(ownsPadding: true)
    }
    private func label(_ key: String) -> String {
        switch key {
        case "leftHandle": "左握把"
        case "rightHandle": "右握把"
        case "leftX": "左摇杆"
        case "rightX": "右摇杆"
        default: MobileShortcutButton(rawValue: key)?.title ?? key
        }
    }
}

struct MobileShortcutControls: View {
    @EnvironmentObject private var settings: AppSettingsStore
    @EnvironmentObject private var streaming: StreamingFeatureStore
    @Environment(\.appPageIsActive) private var active
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.locale) private var locale
    @StateObject private var capture = MobileControllerCaptureSession()
    @State private var selected = MobileShortcutTarget.menu
    @State private var previous: [MobileShortcutButton] = []
    @State private var confirmRestore = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(MobileShortcutTarget.allCases) { target in
                HStack(spacing: 12) {
                    Button {
                        selected = target
                        capture.begin(.shortcut(target), input: streaming.input)
                    } label: {
                        HStack {
                            AppIconView(target == .menu ? .controller : .pulse, size: 22)
                            VStack(alignment: .leading, spacing: 4) {
                                Text(LocalizedStringKey(target.title))
                                Text(verbatim: shortcutTitle(target))
                                    .appFont(.caption).foregroundStyle(
                                        AppThemePalette.textSecondary)
                            }
                            Spacer()
                            Text("A").appFont(.caption)
                        }.frame(maxWidth: .infinity, minHeight: 44).contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("shortcut-record-\(target.rawValue)")
                    .disabled(capture.target != nil)
                    Button("清空", systemImage: "xmark.circle") {
                        settings.mobile.setShortcut([], for: target)
                    }.labelStyle(.iconOnly).frame(width: 44, height: 44).buttonStyle(.plain)
                        .disabled(
                            capture.target != nil
                                || settings.mobile.shortcutButtons(for: target).isEmpty
                        )
                        .accessibilityLabel(Text("\(Text(LocalizedStringKey(target.title)))，清空组合"))
                }
                .padding(.horizontal, AppLayoutPolicy.rowInset)
                .padding(.vertical, 8)
                .background {
                    if selected == target {
                        AppSelectionBackdrop(outline: RoundedRectangle(cornerRadius: 6))
                    } else {
                        Color.clear.appSurface()
                    }
                }
                .accessibilityElement(children: .contain)
                .accessibilityAddTraits(selected == target ? .isSelected : [])
                .accessibilityIdentifier("shortcut-row-\(target.rawValue)")
            }
            if capture.target != nil {
                MobileControllerPicture(highlighted: capture.pressed)
                Button("取消录入") { capture.cancel() }
            }
            if let message = capture.message {
                Text(LocalizedStringKey(message))
                    .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
                    .accessibilityIdentifier("shortcut-capture-status")
            }
            VStack(alignment: .leading, spacing: 4) {
                AppInlineHint("点按或按 A 开始，松开组合后自动保存", identifier: "shortcut-input")
                if capture.target == nil {
                    AppInstructionSteps(steps: ["选择目标", "录入组合", "松开保存"])
                }
                AppInlineHint(
                    "最多 4 键；长按一次触发。菜单关闭后松开按键恢复输入。",
                    identifier: "controller-menu")
            }
            .padding(.horizontal, AppLayoutPolicy.rowInset)
            Divider().overlay(AppThemePalette.border)
            Button("恢复默认快捷操作") { confirmRestore = true }
                .buttonStyle(AppSettingsRowButtonStyle(inset: AppLayoutPolicy.rowInset))
                .disabled(capture.target != nil)
                .appSurface()
        }
        .appSettingsInformation(ownsPadding: true)
        .alert("恢复默认快捷操作？", isPresented: $confirmRestore) {
            Button("取消", role: .cancel) {}
            Button("恢复默认") {
                var preferences = settings.mobile
                preferences.customMenuButtons = nil
                preferences.menuShortcut = .menuView
                preferences.diagnosticsButtons = []
                preferences.menuHoldDuration = 0.6
                settings.mobile = preferences
            }
        }
        .task(id: active && scenePhase == .active) {
            guard active && scenePhase == .active else {
                capture.cancel()
                return
            }
            while !Task.isCancelled {
                let held = MobileShortcutButton.allCases.filter {
                    $0.isPressed(in: streaming.input.rawControllerState)
                }
                if capture.target != nil {
                    capture.poll(input: streaming.input, settings: settings)
                } else {
                    if held.contains(.down) && !previous.contains(.down) { selected = .diagnostics }
                    if held.contains(.up) && !previous.contains(.up) { selected = .menu }
                    if held.contains(.a) && !previous.contains(.a) {
                        capture.begin(.shortcut(selected), input: streaming.input)
                    }
                }
                previous = held
                try? await Task.sleep(for: .milliseconds(8))
            }
            capture.cancel()
        }
        .onDisappear { capture.cancel() }
    }

    private func shortcutTitle(_ target: MobileShortcutTarget) -> String {
        let buttons = settings.mobile.shortcutButtons(for: target)
        return buttons.isEmpty
            ? appLocalizedString("未绑定", locale: locale)
            : buttons.map { appLocalizedString($0.title, locale: locale) }.joined(separator: " + ")
    }
}

/// 明确选择目标后进入录入，释放组合或回中后自动保存。
struct MobileGamepadMappingControls: View {
    @EnvironmentObject private var settings: AppSettingsStore
    @EnvironmentObject private var streaming: StreamingFeatureStore
    @Environment(\.appPageIsActive) private var active
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.locale) private var locale
    @StateObject private var capture = MobileControllerCaptureSession()
    @State private var selected = "a"
    @State private var positiveAxis = true
    @State private var previous: [MobileShortcutButton] = []
    @State private var confirmRestore = false
    private var targets: [String] {
        MobileShortcutButton.allCases.map(\.rawValue) + MobileControlAxis.allCases.map(\.rawValue)
    }
    private var binding: MobileControlBinding {
        settings.mobile.gamepadBindings[streaming.input.controllerLayoutKey]?[selected]
            ?? MobileControlAxis(rawValue: selected).map { .axis($0, inverted: false) }
            ?? .buttons([MobileShortcutButton(rawValue: selected) ?? .a])
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            MobileControllerPicture(
                highlighted: capture.pressed, selected: selected, select: selectTarget
            )
            .allowsHitTesting(capture.target == nil)
            Group {
                if let message = capture.message {
                    Text(LocalizedStringKey(message))
                        .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
                        .accessibilityIdentifier("mapping-capture-status")
                } else {
                    AppInstructionSteps(steps: ["点按目标", "输入按键或轴", "松开保存"])
                }
                AppInlineHint("松开或回中后自动保存，立即生效", identifier: "controller-mapping")
            }
            .padding(.horizontal, AppLayoutPolicy.rowInset)
            Menu {
                ForEach(targets, id: \.self) { key in
                    Button {
                        selectTarget(key)
                    } label: {
                        Text(LocalizedStringKey(title(key)))
                        if key == selected { Image(systemName: "checkmark") }
                    }
                }
            } label: {
                HStack {
                    Text("映射目标")
                    Spacer(minLength: 8)
                    Text(LocalizedStringKey(title(selected))).appFont(.caption).foregroundStyle(
                        AppThemePalette.textSecondary)
                    AppIconView(.chevron, size: 12).rotationEffect(.degrees(90))
                }
                .frame(minHeight: 44).padding(.horizontal, AppLayoutPolicy.rowInset)
            }
            .buttonStyle(.plain).disabled(capture.target != nil)
            .accessibilityLabel("映射目标")
            .accessibilityValue(Text(LocalizedStringKey(title(selected))))
            .accessibilityIdentifier("settings-picker-映射目标")
            .appSurface(radius: AppLayoutPolicy.rowRadius)
            if MobileControlAxis(rawValue: selected) != nil {
                AppSettingsPicker(
                    "录入方向", selection: $positiveAxis,
                    selectedTitle: selected.hasSuffix("X")
                        ? (positiveAxis ? "向右" : "向左") : (positiveAxis ? "向上" : "向下")
                ) {
                    Text(LocalizedStringKey(selected.hasSuffix("X") ? "向右" : "向上")).tag(true)
                    Text(LocalizedStringKey(selected.hasSuffix("X") ? "向左" : "向下")).tag(false)
                }.onChange(of: positiveAxis) { _, _ in selectTarget(selected) }
                    .appSurface(radius: AppLayoutPolicy.rowRadius)
            }
            AppFactGroup(facts: [("当前映射", binding.title(locale: locale))])
                .padding(.horizontal, AppLayoutPolicy.rowInset)
            if capture.target != nil { Button("取消录入") { capture.cancel() } }
            if case .rawButtons(let keys, _) = binding,
                keys.contains(where: { $0.hasPrefix("extra:") }),
                MobileShortcutButton(rawValue: selected) != nil
            {
                AppSettingsToggle(
                    "保留标准按键作为备用",
                    isOn: Binding(
                        get: {
                            if case .rawButtons(_, let any) = binding { return any }
                            return false
                        },
                        set: { any in
                            let sources =
                                any
                                ? Array(Set(keys + [selected])).sorted()
                                : keys.filter { $0 != selected }
                            settings.mobile.gamepadBindings[
                                streaming.input.controllerLayoutKey, default: [:]][selected] =
                                .rawButtons(sources, any: any)
                        }))
            }
            Button("恢复当前手柄默认映射") { confirmRestore = true }
                .disabled(capture.target != nil || !streaming.input.hasController)
                .padding(.horizontal, AppLayoutPolicy.rowInset)
        }
        .appSettingsInformation(ownsPadding: true)
        .alert("恢复当前手柄默认映射？", isPresented: $confirmRestore) {
            Button("取消", role: .cancel) {}
            Button("恢复默认") {
                settings.mobile.gamepadBindings.removeValue(
                    forKey: streaming.input.controllerLayoutKey)
            }
        }
        .task(id: active && scenePhase == .active) {
            guard active && scenePhase == .active else {
                capture.cancel()
                return
            }
            previous = MobileShortcutButton.allCases.filter {
                $0.isPressed(in: streaming.input.rawControllerState)
            }
            while !Task.isCancelled {
                let held = MobileShortcutButton.allCases.filter {
                    $0.isPressed(in: streaming.input.rawControllerState)
                }
                if capture.target != nil {
                    capture.poll(input: streaming.input, settings: settings)
                } else {
                    let index = targets.firstIndex(of: selected) ?? 0
                    if held.contains(.down) && !previous.contains(.down) {
                        selectTarget(targets[(index + 1) % targets.count])
                    }
                    if held.contains(.up) && !previous.contains(.up) {
                        selectTarget(targets[(index + targets.count - 1) % targets.count])
                    }
                    if held.contains(.a) && !previous.contains(.a) {
                        selectTarget(selected)
                    }
                }
                previous = held
                try? await Task.sleep(for: .milliseconds(8))
            }
            capture.cancel()
        }
        .onDisappear { capture.cancel() }
    }
    private func title(_ key: String) -> String {
        MobileShortcutButton(rawValue: key)?.title ?? MobileControlAxis(rawValue: key)?.title ?? key
    }
    private func selectTarget(_ key: String) {
        selected = key
        capture.begin(.control(key, positive: positiveAxis), input: streaming.input)
    }
}

struct MobileControllerDevicePicker: View {
    @ObservedObject var input: StreamInputSession
    var body: some View {
        AppSettingsPicker(
            "输入手柄",
            selection: Binding(
                get: { input.selectedControllerID }, set: { input.selectController(id: $0) }),
            selectedTitle: input.selectedControllerID == "auto"
                ? "自动" : input.controllerName ?? "自动"
        ) {
            Text("自动").tag("auto")
            ForEach(Array(input.availableControllers.enumerated()), id: \.offset) {
                index, controller in
                Text("\(index + 1) · \(controller.vendorName ?? "游戏手柄")").tag(
                    StreamInputSession.controllerID(controller))
            }
        }
    }
}
