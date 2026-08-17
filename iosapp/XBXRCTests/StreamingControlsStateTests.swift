import CoreGraphics
import XCTest
@testable import XBXRC

final class StreamingControlsStateTests: XCTestCase {
    func testTwoFingerInwardGestureRecognizesSymmetricMovement() {
        XCTAssertTrue(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 30, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 130, y: 128),
                    rightEnd: CGPoint(x: 870, y: 124)
                )
            )
        )
    }

    func testTwoFingerInwardGestureRejectsSameDirectionMovement() {
        XCTAssertFalse(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 30, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 130, y: 128),
                    rightEnd: CGPoint(x: 1_000, y: 124)
                )
            )
        )
    }

    func testTwoFingerInwardGestureRejectsVerticalDriftAndLateStart() {
        XCTAssertFalse(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 30, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 130, y: 180),
                    rightEnd: CGPoint(x: 870, y: 124)
                )
            )
        )
        XCTAssertFalse(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 30, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 130, y: 128),
                    rightEnd: CGPoint(x: 870, y: 124),
                    startDeltaMilliseconds: 121
                )
            )
        )
    }

    func testTwoFingerInwardGestureRejectsInvalidStartRegionsAndDuration() {
        XCTAssertFalse(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 260, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 360, y: 128),
                    rightEnd: CGPoint(x: 870, y: 124)
                )
            )
        )
        XCTAssertFalse(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 30, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 130, y: 128),
                    rightEnd: CGPoint(x: 870, y: 124),
                    durationMilliseconds: 451
                )
            )
        )
    }

    func testTwoFingerInwardGestureUsesAdaptiveMinimumTravel() {
        XCTAssertFalse(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 30, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 101, y: 128),
                    rightEnd: CGPoint(x: 899, y: 124)
                )
            )
        )
        XCTAssertTrue(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 30, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 120, y: 128),
                    rightEnd: CGPoint(x: 880, y: 124)
                )
            )
        )
    }

    func testControlsStateReturnsToDockAfterSheet() {
        var state = StreamingControlsState()
        state.reveal()
        XCTAssertEqual(state.surface, .dock)
        state.present(.debugSheet)
        XCTAssertTrue(state.suspendsAutoHide)
        state.dismissPresentedSurface()
        XCTAssertEqual(state.surface, .dock)
        state.hide()
        XCTAssertEqual(state.surface, .hidden)
    }

    func testConnectionSummaryKeepsRawMetricsOutOfUserState() {
        let healthy = diagnostics(rtt: 0.03, jitter: 0.004, frameSupplyDelta: 1)
        XCTAssertEqual(healthy.connectionSummary(featureState: .playing), .good)

        let unstable = diagnostics(rtt: 0.13, jitter: 0.004, frameSupplyDelta: 1)
        XCTAssertEqual(unstable.connectionSummary(featureState: .playing), .unstable)
        XCTAssertEqual(healthy.connectionSummary(featureState: .recovering), .recovering)
    }

    private func sample(
        leftStart: CGPoint,
        rightStart: CGPoint,
        leftEnd: CGPoint,
        rightEnd: CGPoint,
        startDeltaMilliseconds: Double = 20,
        durationMilliseconds: Double = 260
    ) -> StreamingInwardGestureSample {
        StreamingInwardGestureSample(
            leftStart: leftStart,
            rightStart: rightStart,
            leftEnd: leftEnd,
            rightEnd: rightEnd,
            startDeltaMilliseconds: startDeltaMilliseconds,
            durationMilliseconds: durationMilliseconds,
            viewWidth: 1_000
        )
    }

    private func diagnostics(
        rtt: Double,
        jitter: Double,
        frameSupplyDelta: Int64
    ) -> StreamingSessionDiagnostics {
        StreamingSessionDiagnostics(
            peerStats: StreamingPeerStats(
                inboundVideoBytes: 1,
                inboundAudioBytes: 1,
                framesDecoded: 1,
                framesDropped: 0,
                packetsLost: 0,
                roundTripTimeSeconds: rtt,
                jitterSeconds: jitter,
                packetsReceived: 1,
                frameSupplyDelta: frameSupplyDelta
            ),
            displayHealth: nil,
            peer: nil,
            audioLifecycleState: nil,
            audioRoute: nil,
            audioTrackReady: true,
            updatedAtMilliseconds: 1
        )
    }
}
