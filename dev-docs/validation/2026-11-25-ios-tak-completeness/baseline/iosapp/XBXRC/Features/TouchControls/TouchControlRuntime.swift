import CoreGraphics
import Foundation

struct TouchInputContribution: Equatable {
    var buttons: UInt16 = 0
    var axes: [String: Double] = [:]
    var layers: Set<String> = []

    mutating func add(_ action: TouchAction?, allowsLayers: Bool = true) {
        guard let action else { return }
        switch action {
        case .gamepad(let name):
            if let mask = TouchGamepadMapping.masks[name] {
                buttons |= mask
            } else {
                axes[name, default: 0] += 1
            }
        case .layer(let name):
            if allowsLayers { layers.insert(name) }
        case .group(let items):
            for item in items { add(item, allowsLayers: allowsLayers) }
        case .segmented: break
        }
    }
}

enum TouchGamepadMapping {
    static let masks: [String: UInt16] = [
        "guide": 2, "menu": 4, "view": 8,
        "gamepadA": 16, "gamepadB": 32, "gamepadX": 64, "gamepadY": 128,
        "dPadUp": 256, "dPadDown": 512, "dPadLeft": 1024, "dPadRight": 2048,
        "leftBumper": 4096, "rightBumper": 8192, "leftThumb": 16384, "rightThumb": 32768,
    ]
    static func title(_ name: String) -> String {
        [
            "guide": "Xbox", "menu": "菜单", "view": "视图",
            "gamepadA": "A", "gamepadB": "B", "gamepadX": "X", "gamepadY": "Y",
            "leftBumper": "LB", "rightBumper": "RB", "leftThumb": "L3", "rightThumb": "R3",
            "leftTrigger": "LT", "rightTrigger": "RT",
            "dPadUp": "↑", "dPadDown": "↓", "dPadLeft": "←", "dPadRight": "→",
        ][name] ?? name
    }

    static func state(contributions: [TouchInputContribution]) -> IOSStreamGamepadState {
        var state = IOSStreamGamepadState.neutral
        var axes: [String: Double] = [:]
        for contribution in contributions {
            state.buttonMask |= contribution.buttons
            for (name, value) in contribution.axes where value.isFinite {
                axes[name, default: 0] += value
            }
        }
        func axis(_ name: String) -> Float {
            let value = axes[name, default: 0]
            return abs(value) <= 0.00001 ? 0 : Float(min(1, max(-1, value)))
        }
        func thumb(_ prefix: String, dimension: String, positive: String, negative: String) -> Float
        {
            let value =
                axes[prefix + dimension, default: 0]
                + axes[prefix + positive, default: 0] - axes[prefix + negative, default: 0]
            return abs(value) <= 0.00001 ? 0 : Float(min(1, max(-1, value)))
        }
        state.leftThumbX = thumb(
            "leftJoystick", dimension: "X", positive: "Right", negative: "Left")
        state.leftThumbY = thumb("leftJoystick", dimension: "Y", positive: "Up", negative: "Down")
        state.rightThumbX = thumb(
            "rightJoystick", dimension: "X", positive: "Right", negative: "Left")
        state.rightThumbY = thumb("rightJoystick", dimension: "Y", positive: "Up", negative: "Down")
        state.leftTrigger = max(0, axis("leftTrigger"))
        state.rightTrigger = max(0, axis("rightTrigger"))
        return state
    }
}

struct TouchPullState {
    let pointerID: Int
    let offset: CGPoint
    let action: TouchAction
    let segmentIndex: Int?
}

struct TouchControlRuntime {
    var socket: TouchControlSocket
    private(set) var pointers: [Int: CGPoint] = [:]
    private var starts: [Int: CGPoint] = [:]
    private var primaryPointer: Int?
    private var lastMovementTimestamp: TimeInterval?
    private(set) var value = CGPoint.zero
    private(set) var toggled = false

    var isActive: Bool { !pointers.isEmpty || toggled || (socket.control.sticky && value.y > 0) }
    var hasExclusivePointer: Bool {
        ![.button, .arcadeButtons].contains(socket.control.kind) && primaryPointer != nil
    }
    var needsTimeUpdate: Bool { socket.control.kind == .touchpad && value != .zero }
    var activeArcadeButtonIDs: Set<String> {
        Set(pointers.values.flatMap { TouchArcadeGeometry.activeButtonIDs(at: $0, in: socket) })
    }
    var isPulled: Bool { !pullStates.isEmpty }
    /// 输入和提示图共享拉动方向，保证分段素材与实际动作对应。
    var pullStates: [TouchPullState] {
        guard socket.control.kind == .button, let action = socket.control.pullAction else {
            return []
        }
        return pointers.keys.sorted().compactMap { id in
            guard let point = pointers[id], let start = starts[id] else { return nil }
            let delta = CGPoint(x: point.x - start.x, y: point.y - start.y)
            guard hypot(delta.x, delta.y) >= socket.unitSize / 2 else { return nil }
            if case .segmented(let items) = action {
                guard !items.isEmpty else { return nil }
                let slice = 2 * Double.pi / Double(items.count)
                let angle = atan2(Double(delta.y), Double(delta.x))
                let rawIndex = Int(floor((angle + .pi / 2 + slice / 2) / slice))
                let index = (rawIndex % items.count + items.count) % items.count
                return .init(
                    pointerID: id, offset: delta, action: items[index], segmentIndex: index)
            }
            return .init(pointerID: id, offset: delta, action: action, segmentIndex: nil)
        }
    }

    @discardableResult
    mutating func begin(
        pointerID: Int, position: CGPoint,
        timestamp: TimeInterval = ProcessInfo.processInfo.systemUptime
    ) -> Bool {
        guard !hasExclusivePointer else { return false }
        pointers[pointerID] = position
        starts[pointerID] = position
        if primaryPointer == nil { primaryPointer = pointerID }
        if socket.control.kind == .touchpad {
            lastMovementTimestamp = timestamp
            value = .zero
        }
        if socket.control.kind == .directionalPad
            || (!socket.control.relative && socket.control.kind != .button)
        {
            updateValue(position: position, delta: .zero)
        }
        return true
    }

    mutating func move(
        pointerID: Int, position: CGPoint,
        timestamp: TimeInterval = ProcessInfo.processInfo.systemUptime
    ) {
        guard let previous = pointers[pointerID] else { return }
        pointers[pointerID] = position
        if primaryPointer == pointerID {
            if socket.control.kind == .touchpad {
                // 静止期绘制会暂停；限制速度窗口，保持重新拖动时的即时响应。
                let elapsed = min(
                    20, max(1, (timestamp - (lastMovementTimestamp ?? timestamp)) * 1000))
                value = .init(
                    x: (position.x - previous.x) / elapsed,
                    y: (previous.y - position.y) / elapsed)
                lastMovementTimestamp = timestamp
                return
            }
            updateValue(
                position: position,
                delta: .init(x: position.x - previous.x, y: position.y - previous.y))
        }
    }

    mutating func end(pointerID: Int, cancelled: Bool) {
        guard pointers.removeValue(forKey: pointerID) != nil else { return }
        starts.removeValue(forKey: pointerID)
        guard pointers.isEmpty else { return }
        primaryPointer = nil
        lastMovementTimestamp = nil
        switch socket.control.kind {
        case .button:
            if cancelled { toggled = false } else if socket.control.toggle { toggled.toggle() }
        case .throttle:
            if cancelled || !socket.control.sticky || value.y <= 0 { value = .zero }
        default: value = .zero
        }
    }

    mutating func reset() {
        pointers.removeAll()
        starts.removeAll()
        primaryPointer = nil
        lastMovementTimestamp = nil
        value = .zero
        toggled = false
    }

    @discardableResult
    mutating func advance(to timestamp: TimeInterval) -> Bool {
        guard needsTimeUpdate, let lastMovementTimestamp,
            timestamp - lastMovementTimestamp >= 0.02
        else { return false }
        value = .zero
        return true
    }

    func contribution() -> TouchInputContribution {
        var result = TouchInputContribution()
        let control = socket.control
        switch control.kind {
        case .button:
            if isActive { result.add(control.action, allowsLayers: control.enabled) }
            for pull in pullStates {
                result.add(pull.action, allowsLayers: control.enabled)
            }
        case .joystick:
            if !pointers.isEmpty {
                for mapping in control.axis { apply(mapping, to: &result) }
                if hypot(value.x, value.y) >= control.actionThreshold {
                    result.add(control.action, allowsLayers: control.enabled)
                }
            }
        case .directionalPad:
            if !pointers.isEmpty {
                let deadzone = socket.collisionRadius * control.directionalPadDeadzone
                if hypot(value.x, value.y) > deadzone {
                    let exclusive =
                        control.properties["interaction"]?["activationType"]?.string == "exclusive"
                    if exclusive {
                        if abs(value.x) >= abs(value.y) {
                            result.add(.gamepad(value.x >= 0 ? "dPadRight" : "dPadLeft"))
                        } else {
                            result.add(.gamepad(value.y >= 0 ? "dPadUp" : "dPadDown"))
                        }
                    } else {
                        let sector = Int((atan2(value.y, value.x) / (.pi / 4)).rounded())
                        let angle = CGFloat(sector) * .pi / 4
                        if cos(angle) > 0.4 { result.add(.gamepad("dPadRight")) }
                        if cos(angle) < -0.4 { result.add(.gamepad("dPadLeft")) }
                        if sin(angle) > 0.4 { result.add(.gamepad("dPadUp")) }
                        if sin(angle) < -0.4 { result.add(.gamepad("dPadDown")) }
                    }
                }
            }
        case .throttle:
            if isActive {
                for (field, input) in [("axisUp", "axisUp"), ("axisDown", "axisDown")] {
                    if let output = control.properties[field]?.string {
                        apply(
                            .init(
                                input: input, output: output, deadzone: 0,
                                radial: false, sensitivity: 1), to: &result)
                    } else {
                        for mapping in TouchAxisMapping.parse(control.properties[field]) {
                            apply(mapping, to: &result)
                        }
                    }
                }
            }
        case .touchpad:
            if !pointers.isEmpty {
                result.add(control.action, allowsLayers: control.enabled)
                for mapping in control.axis { apply(mapping, to: &result) }
            }
        case .arcadeButtons:
            let active = activeArcadeButtonIDs
            for button in TouchArcadeGeometry.buttons(in: socket) where active.contains(button.id) {
                result.add(
                    button.control.action, allowsLayers: control.enabled && button.control.enabled)
            }
        }
        return result
    }

    private mutating func updateValue(position: CGPoint, delta: CGPoint) {
        let control = socket.control
        switch control.kind {
        case .joystick:
            let halfUnit = socket.unitSize / 2
            let radius =
                control.expand || halfUnit > socket.collisionRadius
                ? min(socket.collisionRadius, max(halfUnit, socket.collisionRadius - halfUnit))
                : halfUnit
            let next =
                control.relative
                ? CGPoint(x: value.x + delta.x / radius, y: value.y - delta.y / radius)
                : CGPoint(
                    x: (position.x - socket.center.x) / radius,
                    y: (socket.center.y - position.y) / radius)
            let constrained = CGPoint(
                x: control.hasHorizontalAxis ? next.x : 0,
                y: control.hasVerticalAxis ? next.y : 0)
            let scale = max(1, hypot(constrained.x, constrained.y))
            value = .init(x: constrained.x / scale, y: constrained.y / scale)
        case .directionalPad:
            value = .init(x: position.x - socket.center.x, y: socket.center.y - position.y)
        case .throttle:
            let next =
                control.relative
                ? value.y - delta.y / socket.collisionRadius
                : (socket.center.y - position.y) / socket.collisionRadius
            value = .init(x: 0, y: min(1, max(-1, next)))
        case .button, .touchpad, .arcadeButtons: break
        }
    }

    private func apply(_ mapping: TouchAxisMapping, to result: inout TouchInputContribution) {
        var vector = value
        let threshold = mapping.deadzone
        func deadzone(_ value: Double) -> Double {
            guard value != 0 else { return 0 }
            let sign = value > 0 ? 1.0 : -1.0
            return value * (1 - threshold) + sign * threshold
        }
        func response(_ value: Double) -> Double {
            let curved = mapping.responseCurve?.apply(value) ?? value
            guard curved != 0 else { return 0 }
            let sign = curved > 0 ? 1.0 : -1.0
            return (curved - sign * threshold) * mapping.sensitivity + sign * threshold
        }
        let x = Double(vector.x)
        let y = Double(vector.y)
        let scalar: Double
        switch mapping.input {
        case "axisX": scalar = x
        case "axisY": scalar = y
        case "axisUp": scalar = max(0, y)
        case "axisDown": scalar = max(0, -y)
        case "axisLeft": scalar = max(0, -x)
        case "axisRight": scalar = max(0, x)
        default: scalar = 0
        }
        if mapping.input == "axisXY" {
            if mapping.radial {
                let length = Double(hypot(vector.x, vector.y))
                if length > 0 {
                    let nextLength = deadzone(length)
                    vector = .init(x: x * nextLength / length, y: y * nextLength / length)
                }
            } else {
                vector = .init(x: deadzone(x), y: deadzone(y))
            }
            result.axes[mapping.output + "X", default: 0] += response(Double(vector.x))
            result.axes[mapping.output + "Y", default: 0] += response(Double(vector.y))
        } else {
            result.axes[mapping.output, default: 0] += response(deadzone(scalar))
        }
    }
}
