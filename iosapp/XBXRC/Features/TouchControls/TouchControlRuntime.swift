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

struct TouchControlRuntime {
    var socket: TouchControlSocket
    private(set) var pointers: [Int: CGPoint] = [:]
    private var starts: [Int: CGPoint] = [:]
    private var primaryPointer: Int?
    private(set) var value = CGPoint.zero
    private(set) var toggled = false

    var isActive: Bool { !pointers.isEmpty || toggled || (socket.control.sticky && value.y > 0) }
    var hasExclusivePointer: Bool { socket.control.kind != .button && primaryPointer != nil }
    var isPulled: Bool {
        pointers.contains { id, point in
            guard let start = starts[id] else { return false }
            return hypot(point.x - start.x, point.y - start.y) >= socket.unitSize / 2
        } && socket.control.pullAction != nil
    }

    @discardableResult
    mutating func begin(pointerID: Int, position: CGPoint) -> Bool {
        guard !hasExclusivePointer else { return false }
        pointers[pointerID] = position
        starts[pointerID] = position
        if primaryPointer == nil { primaryPointer = pointerID }
        if socket.control.kind == .directionalPad
            || (!socket.control.relative && socket.control.kind != .button)
        {
            updateValue(position: position, delta: .zero)
        }
        return true
    }

    mutating func move(pointerID: Int, position: CGPoint) {
        guard let previous = pointers[pointerID] else { return }
        pointers[pointerID] = position
        if primaryPointer == pointerID {
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
        value = .zero
        toggled = false
    }

    func contribution() -> TouchInputContribution {
        var result = TouchInputContribution()
        let control = socket.control
        switch control.kind {
        case .button:
            if isActive { result.add(control.action, allowsLayers: control.enabled) }
            if let pull = control.pullAction {
                for (id, point) in pointers {
                    guard let start = starts[id] else { continue }
                    let delta = CGPoint(x: point.x - start.x, y: point.y - start.y)
                    guard hypot(delta.x, delta.y) >= socket.unitSize / 2 else { continue }
                    if case .segmented(let items) = pull, !items.isEmpty {
                        let slice = 2 * Double.pi / Double(items.count)
                        let angle = atan2(Double(delta.y), Double(delta.x))
                        let rawIndex = Int(floor((angle + .pi / 2 + slice / 2) / slice))
                        let index = (rawIndex % items.count + items.count) % items.count
                        result.add(items[index], allowsLayers: control.enabled)
                    } else {
                        result.add(pull, allowsLayers: control.enabled)
                    }
                }
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
                let deadzone = socket.collisionRadius * 0.23
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
        case .touchpad: break
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
            let scale = max(1, hypot(next.x, next.y))
            value = .init(x: next.x / scale, y: next.y / scale)
        case .directionalPad:
            value = .init(x: position.x - socket.center.x, y: socket.center.y - position.y)
        case .throttle:
            let next =
                control.relative
                ? value.y - delta.y / socket.collisionRadius
                : (socket.center.y - position.y) / socket.collisionRadius
            value = .init(x: 0, y: min(1, max(-1, next)))
        case .button, .touchpad: break
        }
    }

    private func apply(_ mapping: TouchAxisMapping, to result: inout TouchInputContribution) {
        var vector = value
        let threshold = mapping.deadzone
        func transform(_ value: Double) -> Double {
            guard value != 0 else { return 0 }
            let sign = value > 0 ? 1.0 : -1.0
            let offset = value * (1 - threshold) + sign * threshold
            return (offset - sign * threshold) * mapping.sensitivity + sign * threshold
        }
        if mapping.radial {
            let length = Double(hypot(vector.x, vector.y))
            if length > 0 {
                let nextLength = transform(length)
                vector = .init(
                    x: Double(vector.x) * nextLength / length,
                    y: Double(vector.y) * nextLength / length)
            }
        } else {
            vector = .init(x: transform(Double(vector.x)), y: transform(Double(vector.y)))
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
            result.axes[mapping.output + "X", default: 0] += x
            result.axes[mapping.output + "Y", default: 0] += y
        } else {
            result.axes[mapping.output, default: 0] += scalar
        }
    }
}
