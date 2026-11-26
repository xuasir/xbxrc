import Foundation

/// 保留 null、数组槽位和未知字段，供版本归一化与兼容诊断共同消费。
enum TouchJSON: Codable, Equatable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([TouchJSON])
    case object([String: TouchJSON])

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([TouchJSON].self) {
            self = .array(value)
        } else {
            self = .object(try container.decode([String: TouchJSON].self))
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null: try container.encodeNil()
        case .bool(let value): try container.encode(value)
        case .number(let value): try container.encode(value)
        case .string(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
        }
    }

    subscript(key: String) -> TouchJSON? { object?[key] }
    var object: [String: TouchJSON]? { if case .object(let value) = self { value } else { nil } }
    var array: [TouchJSON]? { if case .array(let value) = self { value } else { nil } }
    var string: String? { if case .string(let value) = self { value } else { nil } }
    var number: Double? { if case .number(let value) = self { value } else { nil } }
    var bool: Bool? { if case .bool(let value) = self { value } else { nil } }
}

struct TouchCapabilityIssue: Codable, Equatable, Identifiable, Sendable {
    enum Severity: String, Codable, Sendable { case warning, unsupported }
    let severity: Severity
    let path: String
    let message: String
    var id: String { "\(severity.rawValue):\(path):\(message)" }
}

struct TouchCapabilityReport: Codable, Equatable, Sendable {
    let format: String
    let schema: String
    var controlTypes: [String] = []
    var behaviors: [String] = []
    var issues: [TouchCapabilityIssue] = []
    var isSupported: Bool { issues.allSatisfy { $0.severity != .unsupported } }
    var unsupportedMessages: [String] {
        Array(Set(issues.filter { $0.severity == .unsupported }.map(\.message))).sorted()
    }

    mutating func add(_ severity: TouchCapabilityIssue.Severity, _ path: String, _ message: String)
    {
        issues.append(.init(severity: severity, path: path, message: message))
    }
}

struct TouchLayout: Equatable, Identifiable, Sendable {
    let id: String
    let name: String
    let author: String?
    let content: TouchJSON
    let report: TouchCapabilityReport
}

enum TouchLayoutError: LocalizedError {
    case invalid(String)
    var errorDescription: String? {
        switch self {
        case .invalid(let reason): "布局读取失败：\(reason)"
        }
    }
}

enum TouchControlKind: String, Sendable {
    case button, joystick, directionalPad, throttle, touchpad, arcadeButtons
}

indirect enum TouchAction: Equatable, Sendable {
    case gamepad(String)
    case layer(String)
    case group([TouchAction])
    case segmented([TouchAction])

    static func parse(_ value: TouchJSON?) -> TouchAction? {
        guard let value else { return nil }
        if let name = value.string { return .gamepad(name) }
        if let items = value.array { return .group(items.compactMap { parse($0) }) }
        if value["type"]?.string == "layer", let target = value["target"]?.string {
            return .layer(target)
        }
        if value["type"]?.string == "segmented", let items = value["items"]?.array {
            return .segmented(items.compactMap { parse($0) })
        }
        return nil
    }

    var title: String {
        switch self {
        case .gamepad(let name): TouchGamepadMapping.title(name)
        case .layer: "方向键"
        case .group(let actions): actions.map(\.title).joined(separator: " + ")
        case .segmented: "拖拉"
        }
    }
}

struct TouchResponseCurve: Equatable, Sendable {
    enum Kind: String, Sendable { case circular, circularInverse = "circular-inverse" }
    let kind: Kind
    let lower: Double
    let upper: Double

    static func parse(_ value: TouchJSON?) -> Self? {
        guard let type = value?["type"]?.string, let kind = Kind(rawValue: type),
            let range = value?["range"]?.array, range.count == 2,
            let lower = range[0].number, let upper = range[1].number,
            lower.isFinite, upper.isFinite, (-1...1).contains(lower),
            (-1...1).contains(upper), lower <= upper
        else { return nil }
        return .init(kind: kind, lower: lower, upper: upper)
    }

    func apply(_ value: Double) -> Double {
        guard value.isFinite else { return 0 }
        let magnitude = min(1, abs(value))
        let angle = abs(upper) * .pi / 2
        // TAK 发布实现以 range 上界控制圆弧；零圆弧取线性极限，确保分母有限。
        let inverse = angle < 0.000001 ? magnitude : sin(magnitude * angle) / sin(angle)
        let curved = kind == .circular ? 2 * magnitude - inverse : inverse
        return (value < 0 ? -1 : 1) * min(1, max(0, curved))
    }
}

struct TouchAxisMapping: Equatable, Sendable {
    let input: String
    let output: String
    let deadzone: Double
    let radial: Bool
    let sensitivity: Double
    var responseCurve: TouchResponseCurve? = nil

    static func parse(_ value: TouchJSON?) -> [Self] {
        guard let value else { return [] }
        if let items = value.array { return items.flatMap { parse($0) } }
        guard let input = value["input"]?.string, let output = value["output"]?.string else {
            return []
        }
        return [
            .init(
                input: input, output: output,
                deadzone: min(1, max(0, value["deadzone"]?["threshold"]?.number ?? 0)),
                radial: value["deadzone"]?["radial"]?.bool ?? false,
                sensitivity: min(100, max(0, value["sensitivity"]?.number ?? 1)),
                responseCurve: TouchResponseCurve.parse(value["responseCurve"])
            )
        ]
    }
}

struct TouchControl: Equatable, Sendable {
    let id: String
    let kind: TouchControlKind
    let properties: TouchJSON

    var action: TouchAction? { .parse(properties["action"]) }
    var pullAction: TouchAction? { .parse(properties["pullAction"]) }
    var axis: [TouchAxisMapping] { TouchAxisMapping.parse(properties["axis"]) }
    var scale: Double { min(3, max(0.25, properties["scale"]?.number ?? 1)) }
    var relative: Bool { properties["relative"]?.bool ?? true }
    var expand: Bool { properties["expand"]?.bool ?? true }
    var toggle: Bool { properties["toggle"]?.bool ?? false }
    var sticky: Bool { properties["sticky"]?.bool ?? false }
    var visible: Bool { properties["visible"]?.bool ?? true }
    var enabled: Bool { properties["enabled"]?.bool ?? true }
    var priority: Double { properties["priority"]?.number ?? 0 }
    var actionThreshold: Double { properties["actionThreshold"]?.number ?? 0.5 }
    var directionalPadDeadzone: Double { properties["deadzone"]?.number ?? 0.25 }
    var renderAsButton: Bool { properties["renderAsButton"]?.bool ?? false }
    var hasHorizontalAxis: Bool {
        axis.contains { ["axisXY", "axisX", "axisLeft", "axisRight"].contains($0.input) }
    }
    var hasVerticalAxis: Bool {
        axis.contains { ["axisXY", "axisY", "axisUp", "axisDown"].contains($0.input) }
    }
    var allowsRollover: Bool { kind == .button && !toggle && pullAction == nil }
    var icon: String? {
        let style = properties["styles"]?["default"]
        return style?["faceImage"]?["value"]?.string
            ?? style?["knob"]?["faceImage"]?["value"]?.string
    }
    var title: String {
        switch kind {
        case .directionalPad: "方向键"
        case .joystick:
            axis.contains { $0.output.hasPrefix("left") } ? "左摇杆" : "右摇杆"
        case .throttle: "油门 / 刹车"
        case .touchpad: "触摸板"
        case .arcadeButtons: "格斗按键"
        case .button: action?.title ?? "按键"
        }
    }
}
