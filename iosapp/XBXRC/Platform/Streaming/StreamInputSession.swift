import Combine
import CryptoKit
import Foundation
import GameController

enum StreamInputSource: String, Sendable {
    case controller, touch, idle
    var title: String {
        switch self {
        case .controller: "外接手柄"
        case .touch: "触屏手柄"
        case .idle: "等待输入"
        }
    }
}

enum StreamInputSuppressionReason: Hashable {
    case menu, lifecycle, terminal, settingsCapture, settingsTest, layoutSwitch
}

enum IOSStreamStickTransform {
    static func apply(x: Float, y: Float, deadzone: Double, invertY: Bool) -> (Float, Float) {
        guard x.isFinite, y.isFinite else { return (0, 0) }
        let magnitude = min(1, sqrt(x * x + y * y))
        let zone = Float(min(0.5, max(0, deadzone)))
        guard magnitude > zone else { return (0, 0) }
        let original = sqrt(x * x + y * y)
        let scale = (magnitude - zone) / (1 - zone) / original
        return (x * scale, y * scale * (invertY ? -1 : 1))
    }

    static func apply(_ state: IOSStreamGamepadState, preferences: MobilePreferences)
        -> IOSStreamGamepadState
    {
        var value = state
        let left = apply(
            x: state.leftThumbX, y: state.leftThumbY, deadzone: preferences.leftStickDeadzone,
            invertY: preferences.invertLeftY)
        let right = apply(
            x: state.rightThumbX, y: state.rightThumbY, deadzone: preferences.rightStickDeadzone,
            invertY: preferences.invertRightY)
        value.leftThumbX = left.0
        value.leftThumbY = left.1
        value.rightThumbX = right.0
        value.rightThumbY = right.1
        return value
    }
}

/// 触屏、实体手柄与菜单共用既有 DataChannel 输出，热路径在 MainActor 上采样。
@MainActor
final class StreamInputSession: ObservableObject {
    @Published private(set) var source: StreamInputSource = .idle
    @Published private(set) var menuRevealToken = 0
    @Published private(set) var diagnosticsRevealToken = 0
    @Published private(set) var isSuppressed = false
    @Published private(set) var controllerName: String?
    @Published private(set) var liveState: IOSStreamGamepadState = .neutral
    @Published private(set) var controllerCount = 0
    @Published private(set) var controllerCategory: String?
    @Published private(set) var controllerIsAttachedToDevice: Bool?
    @Published private(set) var controllerBatteryPercent: Int?
    @Published private(set) var controllerSupportsHaptics: Bool?
    @Published private(set) var selectedControllerID = "auto"
    private weak var selectedController: GCController?

    private let preferencesProvider: @MainActor () -> MobilePreferences
    private let controllerProvider: @MainActor () -> GCController?
    private var virtualState = IOSStreamGamepadState.neutral
    private var activeTouches: Set<String> = []
    private var requiresNeutral = false
    private var pendingTouchNeutralFrame = false
    private var suppressionReasons: Set<StreamInputSuppressionReason> = []
    private var shortcuts = MobileShortcutMatcher()
    private var lastShortcutPreferences: MobilePreferences?
    private weak var lastController: GCController?
    private weak var cachedLayoutController: GCController?
    private var cachedLayoutKey = ""
    private var rawPreferencesCache: MobilePreferences?
    private var normalizedPreferencesCache = MobilePreferences()

    struct ControllerSnapshot {
        let controller: GCController?
        let layoutKey: String
        let raw: IOSStreamGamepadState
        let buttons: [String: Float]
        let axes: [String: Float]

        func mapped(using preferences: MobilePreferences) -> IOSStreamGamepadState {
            MobileControlBinding.apply(
                raw, bindings: preferences.gamepadBindings[layoutKey] ?? [:],
                buttons: buttons, axes: axes)
        }
    }

    init(
        preferences: @escaping @MainActor () -> MobilePreferences = { MobilePreferences() },
        controller: @escaping @MainActor () -> GCController? = {
            StreamInputSession.systemController()
        }
    ) {
        preferencesProvider = preferences
        controllerProvider = controller
    }

    var preferences: MobilePreferences {
        let raw = preferencesProvider()
        if rawPreferencesCache != raw {
            rawPreferencesCache = raw
            normalizedPreferencesCache = raw.normalized()
        }
        return normalizedPreferencesCache
    }
    var availableControllers: [GCController] {
        GCController.controllers().filter { $0.extendedGamepad != nil }
    }
    var currentController: GCController? {
        if let selectedController,
            availableControllers.contains(where: { $0 === selectedController })
        {
            return selectedController
        }
        return controllerProvider()
    }
    func selectController(id: String) {
        selectedController = availableControllers.first { Self.controllerID($0) == id }
        selectedControllerID = selectedController == nil ? "auto" : id
        releaseTouches()
        requiresNeutral = true
        shortcuts.reset()
        refreshDeviceStatus()
    }
    static func controllerID(_ controller: GCController) -> String {
        String(ObjectIdentifier(controller).hashValue)
    }

    var rawControllerState: IOSStreamGamepadState { Self.read(currentController?.extendedGamepad) }
    var controllerLayoutKey: String { layoutKey(for: currentController) }

    /// 一个采样同时确定设备、标准输入和附加来源，录入与串流消费同一投影。
    func controllerSnapshot() -> ControllerSnapshot {
        let controller = currentController
        let raw = Self.read(controller?.extendedGamepad)
        return ControllerSnapshot(
            controller: controller, layoutKey: layoutKey(for: controller), raw: raw,
            buttons: Self.readButtons(controller, raw: raw),
            axes: Self.readAxes(controller, raw: raw))
    }

    private func layoutKey(for controller: GCController?) -> String {
        guard let controller else {
            cachedLayoutController = nil
            cachedLayoutKey = ""
            return ""
        }
        if cachedLayoutController === controller { return cachedLayoutKey }
        let layout =
            ([controller.productCategory, controller.vendorName ?? "controller"]
            + controller.physicalInputProfile.buttons.keys.sorted()
            + controller.physicalInputProfile.axes.keys.sorted()).joined(separator: "|")
        cachedLayoutController = controller
        cachedLayoutKey =
            "gc:" + SHA256.hash(data: Data(layout.utf8)).map { String(format: "%02x", $0) }.joined()
        return cachedLayoutKey
    }
    private static func readButtons(_ controller: GCController?, raw: IOSStreamGamepadState)
        -> [String: Float]
    {
        var values = Dictionary(
            uniqueKeysWithValues: MobileShortcutButton.allCases.map {
                ($0.rawValue, $0.value(in: raw))
            })
        guard let controller, let pad = controller.extendedGamepad else { return values }
        let standard: [GCControllerButtonInput?] = [
            pad.buttonA, pad.buttonB, pad.buttonX, pad.buttonY, pad.buttonMenu, pad.buttonOptions,
            pad.buttonHome,
            pad.leftShoulder, pad.rightShoulder, pad.leftTrigger, pad.rightTrigger,
            pad.leftThumbstickButton, pad.rightThumbstickButton, pad.dpad.up, pad.dpad.down,
            pad.dpad.left, pad.dpad.right,
        ]
        var seen = Set(standard.compactMap { $0 }.map { ObjectIdentifier($0) })
        for (key, button) in controller.physicalInputProfile.buttons.sorted(by: { $0.key < $1.key })
        {
            if seen.insert(ObjectIdentifier(button)).inserted {
                values["extra:" + key] = button.value
            }
        }
        return values
    }
    private static func readAxes(_ controller: GCController?, raw: IOSStreamGamepadState)
        -> [String: Float]
    {
        var values = Dictionary(
            uniqueKeysWithValues: MobileControlAxis.allCases.map {
                ($0.rawValue, $0.value(in: raw))
            })
        guard let controller, let pad = controller.extendedGamepad else { return values }
        let standard = [
            pad.leftThumbstick.xAxis, pad.leftThumbstick.yAxis, pad.rightThumbstick.xAxis,
            pad.rightThumbstick.yAxis, pad.dpad.xAxis, pad.dpad.yAxis,
        ]
        var seen = Set(standard.map { ObjectIdentifier($0) })
        for (key, axis) in controller.physicalInputProfile.axes.sorted(by: { $0.key < $1.key }) {
            if seen.insert(ObjectIdentifier(axis)).inserted { values["axis:" + key] = axis.value }
        }
        return values
    }
    private static func systemController() -> GCController? {
        if let current = GCController.current, current.extendedGamepad != nil { return current }
        return GCController.controllers().first { $0.extendedGamepad != nil }
    }
    var hasController: Bool { currentController != nil }
    var showsTouchController: Bool {
        switch preferences.touchControllerMode {
        case .automatic: !hasController
        case .always: true
        case .hidden: false
        }
    }

    func refreshDeviceStatus() {
        let snapshot = controllerSnapshot()
        let controller = snapshot.controller
        controllerName = controller?.vendorName ?? (controller == nil ? nil : "游戏手柄")
        controllerCount = GCController.controllers().filter { $0.extendedGamepad != nil }.count
        controllerCategory = controller?.productCategory
        controllerIsAttachedToDevice = controller?.isAttachedToDevice
        controllerSupportsHaptics = controller.map { $0.haptics != nil }
        if let level = controller?.battery?.batteryLevel, level.isFinite, level >= 0 {
            controllerBatteryPercent = Int((min(1, level) * 100).rounded())
        } else {
            controllerBatteryPercent = nil
        }
        liveState = IOSStreamStickTransform.apply(
            snapshot.mapped(using: preferences), preferences: preferences)
    }

    func outputState(now: Double) -> IOSStreamGamepadState {
        let snapshot = controllerSnapshot()
        let controller = snapshot.controller
        if lastController !== controller {
            lastController = controller
            releaseTouches()
            requiresNeutral = true
            shortcuts.reset()
            refreshDeviceStatus()
        }
        let preferences = self.preferences
        if let previous = lastShortcutPreferences,
            previous.shortcutButtons(for: .menu) != preferences.shortcutButtons(for: .menu)
                || previous.diagnosticsButtons != preferences.diagnosticsButtons
                || previous.menuHoldDuration != preferences.menuHoldDuration
                || previous.gamepadBindings != preferences.gamepadBindings
        {
            shortcuts.reset()
            requiresNeutral = true
        }
        lastShortcutPreferences = preferences
        let rawPhysical = snapshot.raw
        let mapped = snapshot.mapped(using: preferences)
        let physical = IOSStreamStickTransform.apply(mapped, preferences: preferences)
        guard !isSuppressed else { return .neutral }
        if pendingTouchNeutralFrame {
            pendingTouchNeutralFrame = false
            return .neutral
        }
        if requiresNeutral {
            guard snapshot.buttons.values.allSatisfy({ $0 <= 0.5 }),
                MobileShortcutButton.allCases.allSatisfy({ !$0.isPressed(in: mapped) }),
                MobileControlAxis.allCases.allSatisfy({ abs($0.value(in: rawPhysical)) <= 0.15 }),
                MobileControlAxis.allCases.allSatisfy({ abs($0.value(in: mapped)) <= 0.15 })
            else { return .neutral }
            requiresNeutral = false
        }
        let shortcut = shortcuts.sample(mapped, preferences: preferences, now: now)
        if let action = shortcut.action {
            switch action {
            case .menu: menuRevealToken &+= 1
            case .diagnostics: diagnosticsRevealToken &+= 1
            }
        }
        if shortcut.captured { return .neutral }
        let raw: IOSStreamGamepadState
        if physical != .neutral {
            updateSource(.controller)
            raw = physical
        } else if !activeTouches.isEmpty {
            updateSource(.touch)
            raw = virtualState
        } else {
            updateSource(controller == nil ? .idle : .controller)
            raw = .neutral
        }
        return source == .controller
            ? raw : IOSStreamStickTransform.apply(raw, preferences: preferences)
    }

    private func updateSource(_ value: StreamInputSource) {
        if source != value { source = value }
    }

    func setSuppressed(_ value: Bool, reason: StreamInputSuppressionReason = .menu) {
        if value { suppressionReasons.insert(reason) } else { suppressionReasons.remove(reason) }
        let suppressed = !suppressionReasons.isEmpty
        guard suppressed != isSuppressed else { return }
        isSuppressed = suppressed
        releaseTouches()
        requiresNeutral = true
        pendingTouchNeutralFrame = true
        shortcuts.reset()
    }

    func setTouchButton(_ mask: UInt16, pressed: Bool, id: String) {
        guard !isSuppressed else { return }
        if pressed {
            activeTouches.insert(id)
            virtualState.buttonMask |= mask
        } else {
            activeTouches.remove(id)
            virtualState.buttonMask &= ~mask
        }
    }

    func setTouchTrigger(left: Bool, pressed: Bool) {
        guard !isSuppressed else { return }
        let id = left ? "lt" : "rt"
        if pressed { activeTouches.insert(id) } else { activeTouches.remove(id) }
        if left {
            virtualState.leftTrigger = pressed ? 1 : 0
        } else {
            virtualState.rightTrigger = pressed ? 1 : 0
        }
    }

    func setTouchStick(left: Bool, x: Float, y: Float, active: Bool) {
        guard !isSuppressed else { return }
        let id = left ? "ls" : "rs"
        if active { activeTouches.insert(id) } else { activeTouches.remove(id) }
        if left {
            virtualState.leftThumbX = active ? x : 0
            virtualState.leftThumbY = active ? y : 0
        } else {
            virtualState.rightThumbX = active ? x : 0
            virtualState.rightThumbY = active ? y : 0
        }
    }

    func releaseTouches() {
        virtualState = .neutral
        activeTouches.removeAll()
    }

    func releaseTouchesForLayoutChange() {
        releaseTouches()
        // 唯一采样出口先发 neutral，紧接着的新触点保持自己的完整生命周期。
        pendingTouchNeutralFrame = true
    }

    func setTouchLayoutState(_ state: IOSStreamGamepadState, active: Bool) {
        guard !isSuppressed else { return }
        virtualState = state
        if active || state != .neutral {
            activeTouches.insert("layout")
        } else {
            activeTouches.remove("layout")
        }
    }

    func reset() {
        releaseTouches()
        pendingTouchNeutralFrame = false
        suppressionReasons.removeAll()
        isSuppressed = false
        requiresNeutral = true
        shortcuts.reset()
        lastShortcutPreferences = nil
        source = .idle
    }

    static func read(_ gamepad: GCExtendedGamepad?) -> IOSStreamGamepadState {
        guard let gamepad else { return .neutral }
        var mask: UInt16 = 0
        if gamepad.buttonHome?.isPressed == true { mask |= 2 }
        if gamepad.buttonMenu.isPressed { mask |= 4 }
        if gamepad.buttonOptions?.isPressed == true { mask |= 8 }
        if gamepad.buttonA.isPressed { mask |= 16 }
        if gamepad.buttonB.isPressed { mask |= 32 }
        if gamepad.buttonX.isPressed { mask |= 64 }
        if gamepad.buttonY.isPressed { mask |= 128 }
        if gamepad.dpad.up.isPressed { mask |= 256 }
        if gamepad.dpad.down.isPressed { mask |= 512 }
        if gamepad.dpad.left.isPressed { mask |= 1024 }
        if gamepad.dpad.right.isPressed { mask |= 2048 }
        if gamepad.leftShoulder.isPressed { mask |= 4096 }
        if gamepad.rightShoulder.isPressed { mask |= 8192 }
        if gamepad.leftThumbstickButton?.isPressed == true { mask |= 16384 }
        if gamepad.rightThumbstickButton?.isPressed == true { mask |= 32768 }
        return IOSStreamGamepadState(
            buttonMask: mask,
            leftThumbX: gamepad.leftThumbstick.xAxis.value,
            leftThumbY: gamepad.leftThumbstick.yAxis.value,
            rightThumbX: gamepad.rightThumbstick.xAxis.value,
            rightThumbY: gamepad.rightThumbstick.yAxis.value,
            leftTrigger: gamepad.leftTrigger.value,
            rightTrigger: gamepad.rightTrigger.value
        )
    }
}
