import Foundation
import XCTest

@testable import XBXRC

final class StreamingStartupDiagnosticsTests: XCTestCase {
    func testSharedStartupFailureKeepsKindStatusAndRetryability() {
        let error = XboxStreamingError.Startup(
            failure: XboxStartupFailure(
                kind: .auth,
                phase: "resolvingContext", retryable: false, statusCode: 401,
                failureCode: nil, networkErrorKind: nil))
        let payload = StreamingStartupDiagnostics.errorPayload(error)
        XCTAssertEqual(payload["errorKind"], .string("auth"))
        XCTAssertEqual(payload["statusCode"], .integer(401))
        XCTAssertEqual(payload["retryable"], .bool(false))
        XCTAssertFalse(StreamingStartupDiagnostics.isRetryable(error, fallback: true))
        XCTAssertEqual(
            StreamingStartupDiagnostics.userMessage(error, fallback: "failed"), "Xbox 登录状态需要更新")
    }

    func testQueryTimeoutAndBridgeCancellationKeepStartupFacts() {
        let timeout = XboxBridgeError.Query(
            failure: XboxQueryFailure(
                kind: .timeout,
                statusCode: nil, retriable: true, retryAfterSeconds: nil))
        XCTAssertTrue(StreamingStartupDiagnostics.isRetryable(timeout, fallback: false))
        XCTAssertEqual(StreamingStartupDiagnostics.errorPayload(timeout)["timeout"], .bool(true))
        XCTAssertTrue(SectionRequestFailure.isCancellation(XboxBridgeError.Cancelled))
        XCTAssertFalse(
            StreamingStartupDiagnostics.isRetryable(XboxBridgeError.Cancelled, fallback: true))
        XCTAssertEqual(
            StreamingStartupDiagnostics.errorPayload(
                StreamingRuntimeError.cloudEntitlementUnconfirmed)["runtimeErrorCode"],
            .string("cloudEntitlementUnconfirmed"))
    }
    func testRemoteConnectFailureRetainsCategoryAndProtectsPrivateDetails() throws {
        let payload = StreamingStartupDiagnostics.errorPayload(
            XboxStreamingError.Remote(
                "Network error: connection failed; targetId=private-host; sessionId=private-session; https://private.example/path Bearer private-credential"
            )
        )
        XCTAssertEqual(payload["errorType"], .string("XboxStreamingError"))
        XCTAssertEqual(payload["errorKind"], .string("network"))
        XCTAssertEqual(payload["networkErrorKind"], .string("connect"))
        XCTAssertEqual(payload["timeout"], .bool(false))
        try assertPrivateDetailsAbsent(payload)
    }

    func testHTTPRegistrationFailureRetainsStatusAndFixedHint() throws {
        let payload = StreamingStartupDiagnostics.errorPayload(
            XboxStreamingError.Http(
                503,
                "ServerNeverRegistered:targetId=private-host; hint=hostRegistrationRetryExhausted"
            )
        )
        XCTAssertEqual(payload["errorKind"], .string("http"))
        XCTAssertEqual(payload["statusCode"], .integer(503))
        XCTAssertEqual(payload["startupFailureCode"], .string("hostRegistrationRetryExhausted"))
        XCTAssertEqual(payload["serviceErrorCode"], .string("ServerNeverRegistered"))
        try assertPrivateDetailsAbsent(payload)
    }

    func testWakeFailureRetainsHostReadinessCode() throws {
        let payload = StreamingStartupDiagnostics.errorPayload(
            XboxStreamingError.Remote(
                "remoteConsoleWakeCircuitOpen:targetId=private-host;powerState=ConnectedStandby"
            )
        )
        XCTAssertEqual(payload["startupFailureCode"], .string("hostRemotePlayUnavailable"))
        try assertPrivateDetailsAbsent(payload)
    }

    func testTypedHTTPStatusKeepsAuthorityOverServiceMessage() {
        let payload = StreamingStartupDiagnostics.errorPayload(
            XboxStreamingError.Http(503, "Network error: connection failed")
        )
        XCTAssertEqual(payload["errorKind"], .string("http"))
        XCTAssertEqual(payload["statusCode"], .integer(503))
        XCTAssertNil(payload["networkErrorKind"])
    }

    func testDiscoveryTimeoutRetainsItsOwnStageCode() {
        let payload = StreamingStartupDiagnostics.errorPayload(
            XboxStreamingError.Remote("Parse error: 主机跨区域查询超时，请重试")
        )
        XCTAssertEqual(payload["errorKind"], .string("remote"))
        XCTAssertEqual(payload["startupFailureCode"], .string("homeDiscoveryTimeout"))
    }

    func testUnknownRemoteFailureKeepsOnlySafeClassification() throws {
        let payload = StreamingStartupDiagnostics.errorPayload(
            XboxStreamingError.Remote("private-session private-host private-credential")
        )
        XCTAssertEqual(payload["errorKind"], .string("remote"))
        XCTAssertNil(payload["error"])
        XCTAssertNil(payload["startupFailureCode"])
        try assertPrivateDetailsAbsent(payload)
    }

    func testNativeNegotiationTimeoutRetainsRuntimeErrorCode() {
        let payload = StreamingStartupDiagnostics.errorPayload(
            StreamingRuntimeError.remoteDescriptionTimedOut
        )
        XCTAssertEqual(payload["errorKind"], .string("runtime"))
        XCTAssertEqual(payload["runtimeErrorCode"], .string("remoteDescriptionTimedOut"))
    }

    func testFoundationTimeoutAndCancellationKeepDistinctCategories() {
        let timeout = StreamingStartupDiagnostics.errorPayload(
            NSError(domain: NSURLErrorDomain, code: NSURLErrorTimedOut)
        )
        XCTAssertEqual(timeout["networkErrorKind"], .string("timeout"))
        XCTAssertEqual(timeout["timeout"], .bool(true))
        let cancelled = StreamingStartupDiagnostics.errorPayload(CancellationError())
        XCTAssertEqual(cancelled["errorKind"], .string("cancelled"))
    }

    func testStartupProgressRetainsQueueAndRetryFacts() {
        let payload = StreamingStartupDiagnostics.progressPayload(
            StreamingStartupProgress(
                phase: "waitingSessionReady", streamState: "Provisioning",
                totalWaitSeconds: 60, allocationWaitSeconds: 10,
                provisioningWaitSeconds: 50, retryCount: 1, retryLimit: 2
            )
        )
        XCTAssertEqual(payload["phase"], .string("waitingSessionReady"))
        XCTAssertEqual(payload["streamState"], .string("Provisioning"))
        XCTAssertEqual(payload["totalWaitSeconds"], .integer(60))
        XCTAssertEqual(payload["retryCount"], .integer(1))
        XCTAssertEqual(payload["retryLimit"], .integer(2))
    }

    func testStartupProgressRejectsUnknownTextAndClampsWaitValues() throws {
        let payload = StreamingStartupDiagnostics.progressPayload(
            StreamingStartupProgress(
                phase: "private-host", status: "private-credential", streamState: "private-session",
                totalWaitSeconds: UInt64.max
            )
        )
        XCTAssertEqual(payload["phase"], .string("unknown"))
        XCTAssertEqual(payload["status"], .string("unknown"))
        XCTAssertEqual(payload["streamState"], .null)
        XCTAssertEqual(payload["totalWaitSeconds"], .integer(Int64.max))
        try assertPrivateDetailsAbsent(payload)
    }

    private func assertPrivateDetailsAbsent(
        _ payload: [String: IOSRuntimeTraceValue],
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws {
        let text = String(decoding: try JSONEncoder().encode(payload), as: UTF8.self)
        for value in [
            "private-host", "private-session", "private-credential", "https://", "Bearer",
        ] {
            XCTAssertFalse(text.contains(value), file: file, line: line)
        }
    }
}
