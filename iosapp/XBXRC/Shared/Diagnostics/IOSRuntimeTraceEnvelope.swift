import Foundation

enum IOSRuntimeTraceProfile: String, Codable, CaseIterable, Identifiable, Sendable {
    case off
    case production
    case dev

    var id: String { rawValue }

    var title: String {
        switch self {
        case .off: "关闭"
        case .production: "生产"
        case .dev: "开发"
        }
    }
}

enum IOSRuntimeTraceCategory: String, Codable, Sendable {
    case event
    case decision
    case state
    case snapshot
    case log
}

enum IOSRuntimeTraceDimension: String, Codable, CaseIterable, Sendable {
    case core
    case lifecycle
    case network
    case recovery
    case mediaSupply = "media_supply"
    case presentation
    case input
    case nativeVideo = "native_video"
    case frontend
    case engineLog = "engine_log"
}

enum IOSRuntimeTraceImportance: String, Codable, Comparable, Sendable {
    case essential
    case key
    case debug
    case raw

    private var rank: Int {
        switch self {
        case .essential: 0
        case .key: 1
        case .debug: 2
        case .raw: 3
        }
    }

    static func < (lhs: Self, rhs: Self) -> Bool {
        lhs.rank < rhs.rank
    }
}

enum IOSRuntimeTraceValue: Codable, Equatable, Sendable {
    case string(String)
    case integer(Int64)
    case double(Double)
    case bool(Bool)
    case array([IOSRuntimeTraceValue])
    case object([String: IOSRuntimeTraceValue])
    case null

    init(from decoder: any Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Int64.self) {
            self = .integer(value)
        } else if let value = try? container.decode(Double.self) {
            self = .double(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([IOSRuntimeTraceValue].self) {
            self = .array(value)
        } else {
            self = .object(try container.decode([String: IOSRuntimeTraceValue].self))
        }
    }

    func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case let .string(value): try container.encode(value)
        case let .integer(value): try container.encode(value)
        case let .double(value): try container.encode(value)
        case let .bool(value): try container.encode(value)
        case let .array(value): try container.encode(value)
        case let .object(value): try container.encode(value)
        case .null: try container.encodeNil()
        }
    }
}

extension IOSRuntimeTraceValue: ExpressibleByStringLiteral {
    init(stringLiteral value: String) {
        self = .string(value)
    }
}

extension IOSRuntimeTraceValue: ExpressibleByIntegerLiteral {
    init(integerLiteral value: Int64) {
        self = .integer(value)
    }
}

extension IOSRuntimeTraceValue: ExpressibleByFloatLiteral {
    init(floatLiteral value: Double) {
        self = .double(value)
    }
}

extension IOSRuntimeTraceValue: ExpressibleByBooleanLiteral {
    init(booleanLiteral value: Bool) {
        self = .bool(value)
    }
}

struct IOSRuntimeTraceEnvelope: Codable, Equatable, Sendable {
    static let schemaVersion = 3

    let schemaVersion: Int
    let seq: UInt64
    let tsMs: UInt64
    let traceMode: String
    let traceProfile: String
    let dimension: String
    let importance: String
    let category: String
    let domain: String
    let event: String
    let sessionId: String?
    let payload: [String: IOSRuntimeTraceValue]
}

struct IOSRuntimeTraceDraft: Sendable {
    let category: IOSRuntimeTraceCategory
    let domain: String
    let event: String
    let payload: [String: IOSRuntimeTraceValue]
    let dimension: IOSRuntimeTraceDimension
    let importance: IOSRuntimeTraceImportance
    let operationID: String?
}
