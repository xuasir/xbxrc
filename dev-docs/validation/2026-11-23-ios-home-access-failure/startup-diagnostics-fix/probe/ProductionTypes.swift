import Foundation

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
        case .string(let value): try container.encode(value)
        case .integer(let value): try container.encode(value)
        case .double(let value): try container.encode(value)
        case .bool(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
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

enum XboxStreamingError: Swift.Error, Equatable, Hashable, Foundation.LocalizedError {



    case InvalidArgument(String
    )
    case InvalidState(String
    )
    case Cancelled(UInt64
    )
    case Authentication(String
    )
    case Network(String
    )
    case Http(UInt16,String
    )
    case Parse(String
    )
    case Remote(String
    )






    public var errorDescription: String? {
        String(reflecting: self)
    }

}

enum StreamingRuntimeError: LocalizedError, Equatable {
    case invalidStreamTitleID
    case cloudGameUnavailable
    case cloudEntitlementRequired
    case superseded
    case peerConnectionCreationFailed
    case missingLocalDescription
    case missingRemoteDescription
    case remoteDescriptionTimedOut
    case microphonePermissionDenied
    case microphoneUnavailable
    case mediaNegotiationBusy
    case peerConnectionFailed
    case peerConnectionClosed

    var errorDescription: String? {
        switch self {
        case .invalidStreamTitleID: "串流目标标识无效"
        case .cloudGameUnavailable: "当前云目录暂未提供此游戏的可用启动身份"
        case .cloudEntitlementRequired: "当前账户需要此游戏的云游戏访问权限"
        case .superseded: "串流请求已被新的请求替代"
        case .peerConnectionCreationFailed: "创建 WebRTC 连接失败"
        case .missingLocalDescription: "WebRTC 本地协商描述缺失"
        case .missingRemoteDescription: "Xbox 远端协商描述缺失"
        case .remoteDescriptionTimedOut: "应用 Xbox 远端协商描述超时"
        case .microphonePermissionDenied: "麦克风权限未授权"
        case .microphoneUnavailable: "当前设备无法使用麦克风"
        case .mediaNegotiationBusy: "音视频协商正在进行"
        case .peerConnectionFailed: "WebRTC 连接已经失败"
        case .peerConnectionClosed: "WebRTC 连接已经关闭"
        }
    }
}

struct StreamingStartupProgress: Equatable, Sendable {
    let phase: String
    var status = "entered"
    var streamState: String?
    var totalWaitSeconds: UInt64?
    var allocationWaitSeconds: UInt64?
    var provisioningWaitSeconds: UInt64?
    var retryCount: UInt8 = 0
    var retryLimit: UInt8 = 0

}

