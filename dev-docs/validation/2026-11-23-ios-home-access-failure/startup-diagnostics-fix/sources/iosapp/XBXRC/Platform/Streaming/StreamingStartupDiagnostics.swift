import Foundation

enum StreamingStartupDiagnostics {
    static func errorPayload(_ error: any Error) -> [String: IOSRuntimeTraceValue] {
        var payload: [String: IOSRuntimeTraceValue] = [
            "errorType": .string(String(describing: type(of: error)))
        ]
        if let error = error as? XboxStreamingError {
            let message: String?
            var classifiesNetworkMessage = false
            switch error {
            case .InvalidArgument:
                payload["errorKind"] = "invalidArgument"
                message = nil
            case .InvalidState:
                payload["errorKind"] = "invalidState"
                message = nil
            case .Cancelled:
                payload["errorKind"] = "cancelled"
                message = nil
            case .Authentication:
                payload["errorKind"] = "auth"
                message = nil
            case .Network(let value):
                payload["errorKind"] = "network"
                message = value
                classifiesNetworkMessage = true
            case .Http(let status, let value):
                payload["errorKind"] = "http"
                payload["statusCode"] = .integer(Int64(status))
                message = value
            case .Parse(let value):
                payload["errorKind"] = "parse"
                message = value
            case .Remote(let value):
                payload["errorKind"] = "remote"
                message = value
                classifiesNetworkMessage = true
            }
            payload["bridgeErrorKind"] = payload["errorKind"]
            // 错误正文仅用于固定分类；身份、凭据和完整地址留在错误对象中。
            if let message {
                if classifiesNetworkMessage, let kind = networkKind(in: message) {
                    payload["errorKind"] = "network"
                    payload["networkErrorKind"] = .string(kind)
                    payload["timeout"] = .bool(kind == "timeout")
                }
                if let code = startupFailureCode(in: message) {
                    payload["startupFailureCode"] = .string(code)
                }
                if message.localizedCaseInsensitiveContains("ServerNeverRegistered") {
                    payload["serviceErrorCode"] = "ServerNeverRegistered"
                }
            }
        } else if let error = error as? StreamingRuntimeError {
            payload["errorKind"] = "runtime"
            payload["runtimeErrorCode"] = .string(runtimeCode(error))
        } else if error is CancellationError {
            payload["errorKind"] = "cancelled"
        } else {
            let nsError = error as NSError
            if nsError.domain == NSURLErrorDomain {
                payload["errorKind"] = "network"
                let kind: String
                switch nsError.code {
                case NSURLErrorTimedOut: kind = "timeout"
                case NSURLErrorCannotFindHost, NSURLErrorDNSLookupFailed: kind = "dns"
                case NSURLErrorCannotConnectToHost: kind = "connect"
                case NSURLErrorSecureConnectionFailed: kind = "tls"
                default: kind = "unknown"
                }
                payload["networkErrorKind"] = .string(kind)
                payload["timeout"] = .bool(kind == "timeout")
            } else {
                payload["errorKind"] = "unknown"
            }
        }
        return payload
    }

    static func progressPayload(
        _ progress: StreamingStartupProgress
    ) -> [String: IOSRuntimeTraceValue] {
        let phases = [
            "checkingGame", "preparingConsole", "resolvingContext", "creatingSession",
            "waitingSessionReady", "startingRuntime", "ready", "failed",
        ]
        let statuses = ["entered", "succeeded", "failed"]
        let states = [
            "Queued", "Provisioning", "ReadyToConnect", "Provisioned", "Failed",
            "WaitingForResources",
        ]
        return [
            "phase": .string(phases.contains(progress.phase) ? progress.phase : "unknown"),
            "status": .string(statuses.contains(progress.status) ? progress.status : "unknown"),
            "streamState": progress.streamState.flatMap { states.contains($0) ? .string($0) : nil }
                ?? .null,
            "totalWaitSeconds": integer(progress.totalWaitSeconds),
            "allocationWaitSeconds": integer(progress.allocationWaitSeconds),
            "provisioningWaitSeconds": integer(progress.provisioningWaitSeconds),
            "retryCount": .integer(Int64(progress.retryCount)),
            "retryLimit": .integer(Int64(progress.retryLimit)),
        ]
    }

    private static func integer(_ value: UInt64?) -> IOSRuntimeTraceValue {
        value.map { .integer(Int64(clamping: $0)) } ?? .null
    }

    private static func networkKind(in message: String) -> String? {
        let kinds = [
            ("DNS resolution failed", "dns"),
            ("TLS connection failed", "tls"),
            ("request timed out", "timeout"),
            ("connection failed", "connect"),
            ("response body interrupted", "body"),
            ("request initialization failed", "request"),
            ("network request failed", "unknown"),
        ]
        return kinds.first { message.localizedCaseInsensitiveContains($0.0) }?.1
    }

    private static func startupFailureCode(in message: String) -> String? {
        let codes = [
            ("hostRegistrationRetryExhausted", "hostRegistrationRetryExhausted"),
            ("hostRemotePlayUnavailable", "hostRemotePlayUnavailable"),
            ("remoteConsoleWakeCircuitOpen", "hostRemotePlayUnavailable"),
            ("remoteConsoleNotReady", "hostRemotePlayUnavailable"),
            ("homeConsoleLookupTimeout", "homeConsoleLookupTimeout"),
            ("homeTargetContextMissing", "homeTargetContextMissing"),
            ("homeCommandIdentityMissing", "homeCommandIdentityMissing"),
            ("homeDiscoveryCredentialRegionChanged", "homeDiscoveryCredentialRegionChanged"),
            ("homeRecreateReusedSession", "homeRecreateReusedSession"),
            ("streamingStartTimeout", "streamingStartTimeout"),
            ("主机跨区域查询超时", "homeDiscoveryTimeout"),
            ("主机区域查询超时", "homeRegionQueryTimeout"),
            ("主机可用区域暂不可用", "homeRegionsUnavailable"),
        ]
        return codes.first { message.localizedCaseInsensitiveContains($0.0) }?.1
    }

    private static func runtimeCode(_ error: StreamingRuntimeError) -> String {
        switch error {
        case .invalidStreamTitleID: "invalidStreamTitleID"
        case .cloudGameUnavailable: "cloudGameUnavailable"
        case .cloudEntitlementRequired: "cloudEntitlementRequired"
        case .superseded: "superseded"
        case .peerConnectionCreationFailed: "peerConnectionCreationFailed"
        case .missingLocalDescription: "missingLocalDescription"
        case .missingRemoteDescription: "missingRemoteDescription"
        case .remoteDescriptionTimedOut: "remoteDescriptionTimedOut"
        case .microphonePermissionDenied: "microphonePermissionDenied"
        case .microphoneUnavailable: "microphoneUnavailable"
        case .mediaNegotiationBusy: "mediaNegotiationBusy"
        case .peerConnectionFailed: "peerConnectionFailed"
        case .peerConnectionClosed: "peerConnectionClosed"
        }
    }
}
