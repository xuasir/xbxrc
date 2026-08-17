import Foundation

enum CloudLibraryDiagnostics {
    static func elapsedMilliseconds(since start: Date) -> Int {
        max(0, Int(Date().timeIntervalSince(start) * 1_000))
    }

    static func errorPayload(
        _ error: any Error,
        extra: [String: IOSRuntimeTraceValue] = [:]
    ) -> [String: IOSRuntimeTraceValue] {
        let nsError = error as NSError
        let safeMessage = safeError(error)
        var payload = extra
        payload["errorDomain"] = .string(nsError.domain)
        payload["errorCode"] = .integer(Int64(nsError.code))
        payload["errorType"] = .string(String(describing: type(of: error)))
        payload["errorKind"] = .string(classifyError(safeMessage))
        payload["timeout"] = .bool(isTimeout(nsError: nsError, message: safeMessage))
        payload["retriable"] = .bool(isRetriable(nsError: nsError, message: safeMessage))
        payload["error"] = .string(safeMessage)
        if let statusCode = statusCode(in: safeMessage) {
            payload["statusCode"] = .integer(Int64(statusCode))
        }
        if let offerings = capturedValues(named: "offering", in: safeMessage), !offerings.isEmpty {
            payload["offerings"] = .string(offerings.joined(separator: ","))
        }
        if let forceRegionApplied = booleanValue(named: "forceRegionApplied", in: safeMessage) {
            payload["forceRegionApplied"] = .bool(forceRegionApplied)
        }
        return payload
    }

    static func safeError(_ error: any Error) -> String {
        let nsError = error as NSError
        var value = "\(nsError.domain)#\(nsError.code): \(nsError.localizedDescription)"
        let patterns = [
            #"https?://[^\s,;]+"#,
            #"(?i)(bearer|gstoken|refresh[_ -]?token)[=: ]+[^\s,;}]+"#,
            #"cloud-[0-9a-fA-F]{16}"#,
        ]
        for pattern in patterns {
            value = value.replacingOccurrences(
                of: pattern,
                with: "<redacted>",
                options: .regularExpression
            )
        }
        return value
    }

    private static func classifyError(_ message: String) -> String {
        let normalized = message.lowercased()
        if normalized.contains("xgpuwebf2p") {
            return "xgpuwebf2p"
        }
        if normalized.contains("xgpuweb") {
            return "xgpuweb"
        }
        if normalized.contains("xcloud token") {
            return "xcloudTokenUnavailable"
        }
        if normalized.contains("cancel") || normalized.contains("取消") {
            return "cancelled"
        }
        if normalized.contains("timed out") || normalized.contains("timeout") {
            return "timeout"
        }
        return "requestFailed"
    }

    private static func isTimeout(nsError: NSError, message: String) -> Bool {
        let projected = capturedValues(named: "timeout", in: message)
        if let projected, !projected.isEmpty {
            return projected.contains { $0.caseInsensitiveCompare("true") == .orderedSame }
        }
        return nsError.domain == NSURLErrorDomain && nsError.code == NSURLErrorTimedOut
            || message.localizedCaseInsensitiveContains("timeout")
            || message.localizedCaseInsensitiveContains("timed out")
    }

    private static func isRetriable(nsError: NSError, message: String) -> Bool {
        let projected = capturedValues(named: "retriable", in: message)
        if let projected, !projected.isEmpty {
            return projected.contains { $0.caseInsensitiveCompare("true") == .orderedSame }
        }
        if nsError.domain == NSURLErrorDomain {
            return true
        }
        guard let statusCode = statusCode(in: message) else {
            return isTimeout(nsError: nsError, message: message)
        }
        return statusCode == 408 || statusCode == 429 || statusCode >= 500
    }

    private static func statusCode(in message: String) -> Int? {
        let pattern = #"(?i)(?:status(?:Code)?|http)[^0-9]{0,12}([1-5][0-9]{2})"#
        guard let expression = try? NSRegularExpression(pattern: pattern),
              let match = expression.firstMatch(
                  in: message,
                  range: NSRange(message.startIndex..., in: message)
              ),
              let range = Range(match.range(at: 1), in: message)
        else {
            return nil
        }
        return Int(message[range])
    }

    private static func capturedValues(named name: String, in message: String) -> [String]? {
        let pattern = "(?i)\\b\(name)=([a-z0-9_-]+)"
        guard let expression = try? NSRegularExpression(pattern: pattern) else {
            return nil
        }
        return expression.matches(
            in: message,
            range: NSRange(message.startIndex..., in: message)
        ).compactMap { match in
            guard let range = Range(match.range(at: 1), in: message) else { return nil }
            return String(message[range])
        }
    }

    private static func booleanValue(named name: String, in message: String) -> Bool? {
        guard let value = capturedValues(named: name, in: message)?.first else { return nil }
        switch value.lowercased() {
        case "true": return true
        case "false": return false
        default: return nil
        }
    }
}
