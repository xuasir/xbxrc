import CryptoKit
import Foundation

struct IOSRuntimeTraceBudget: Equatable, Sendable {
    let maxFileBytes: UInt64
    let maxFiles: Int
}

enum IOSRuntimeTracePolicy {
    static let profileDefaultsKey = "ios_runtime_trace_profile"
    static let pendingRowLimit = 4_096
    static let batchRowLimit = 128
    static let flushInterval: TimeInterval = 0.04
    static let budgetNoticeIntervalMs: UInt64 = 60_000

    static var defaultProfile: IOSRuntimeTraceProfile {
        #if DEBUG
        .dev
        #else
        .production
        #endif
    }

    static func effectiveProfile(_ stored: IOSRuntimeTraceProfile) -> IOSRuntimeTraceProfile {
        #if DEBUG
        stored
        #else
        stored == .dev ? .production : stored
        #endif
    }

    static func budget(for profile: IOSRuntimeTraceProfile) -> IOSRuntimeTraceBudget {
        switch profile {
        case .off:
            IOSRuntimeTraceBudget(maxFileBytes: 0, maxFiles: 0)
        case .production:
            IOSRuntimeTraceBudget(maxFileBytes: 8 * 1_024 * 1_024, maxFiles: 4)
        case .dev:
            IOSRuntimeTraceBudget(maxFileBytes: 32 * 1_024 * 1_024, maxFiles: 6)
        }
    }

    static func shouldRecord(
        profile: IOSRuntimeTraceProfile,
        importance: IOSRuntimeTraceImportance
    ) -> Bool {
        switch profile {
        case .off:
            false
        case .production:
            importance <= .key
        case .dev:
            true
        }
    }
}

enum IOSRuntimeTraceRedactor {
    private static let sensitiveFragments = [
        "token", "seed", "jwk", "handle", "oauth", "authorization", "callbackurl",
        "accountid", "xuid", "xid", "uhs", "refreshcode",
    ]

    static func redact(
        payload: [String: IOSRuntimeTraceValue],
        profile: IOSRuntimeTraceProfile,
        fingerprintSalt: String
    ) -> [String: IOSRuntimeTraceValue] {
        Dictionary(uniqueKeysWithValues: payload.map { key, value in
            let normalizedKey = key.lowercased().replacingOccurrences(of: "_", with: "")
            if sensitiveFragments.contains(where: normalizedKey.contains) {
                if case .bool = value {
                    return (key, value)
                }
                return (key, .string("<redacted>"))
            }
            if normalizedKey == "productid" {
                return (
                    key,
                    profile == .dev
                        ? redactValue(value, profile: profile, salt: fingerprintSalt)
                        : fingerprintValue(value, salt: fingerprintSalt)
                )
            }
            if normalizedKey == "streamtitleid" || normalizedKey == "xboxtitleid" {
                return (key, fingerprintValue(value, salt: fingerprintSalt))
            }
            return (key, redactValue(value, profile: profile, salt: fingerprintSalt))
        })
    }

    static func sanitizeErrorMessage(_ value: String) -> String {
        redactString(value)
    }

    private static func redactValue(
        _ value: IOSRuntimeTraceValue,
        profile: IOSRuntimeTraceProfile,
        salt: String
    ) -> IOSRuntimeTraceValue {
        switch value {
        case let .string(value):
            .string(redactString(value))
        case let .array(values):
            .array(values.map { redactValue($0, profile: profile, salt: salt) })
        case let .object(values):
            .object(redact(payload: values, profile: profile, fingerprintSalt: salt))
        default:
            value
        }
    }

    private static func fingerprintValue(
        _ value: IOSRuntimeTraceValue,
        salt: String
    ) -> IOSRuntimeTraceValue {
        guard case let .string(raw) = value else {
            return .string("<redacted>")
        }
        let digest = SHA256.hash(data: Data((salt + raw).utf8))
        return .string(digest.prefix(8).map { String(format: "%02x", $0) }.joined())
    }

    private static func redactString(_ value: String) -> String {
        var result = value
        let replacements = [
            (#"https?://[^\s,;]+"#, "<url>"),
            (#"(?i)(bearer|gstoken|refresh[_ -]?token)[=: ]+[^\s,;}]+"#, "<redacted>"),
            (#"cloud-[0-9a-fA-F]{16}"#, "<redacted>"),
            (#"ms-xal-[^\s,;]+"#, "<callback>"),
        ]
        for (pattern, replacement) in replacements {
            result = result.replacingOccurrences(
                of: pattern,
                with: replacement,
                options: .regularExpression
            )
        }
        return result
    }
}
