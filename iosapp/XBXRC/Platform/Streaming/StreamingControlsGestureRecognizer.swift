import UIKit

enum StreamingControlsRevealSource: String, Sendable {
    case singleTap
    case twoFingerInward
    case accessibility
}

final class StreamingInwardGestureRecognizer: UIGestureRecognizer {
    private struct TouchStart {
        let point: CGPoint
        let timestamp: TimeInterval
    }

    private var starts: [ObjectIdentifier: TouchStart] = [:]
    private var latest: [ObjectIdentifier: CGPoint] = [:]
    private var latestTimestamp: TimeInterval = 0
    private var didTraceStart = false
    private var didTraceTerminal = false

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        guard state == .possible else { return }
        for touch in touches {
            let id = ObjectIdentifier(touch)
            starts[id] = TouchStart(point: touch.location(in: view), timestamp: touch.timestamp)
            latest[id] = touch.location(in: view)
            latestTimestamp = max(latestTimestamp, touch.timestamp)
        }
        if starts.count == 2, !didTraceStart {
            didTraceStart = true
            trace("streamControlsGestureStarted", payload: [:])
            validateStartContract()
        }
        if starts.count > 2 { reject(reason: "tooManyTouches") }
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        guard state == .possible else { return }
        for touch in touches {
            latest[ObjectIdentifier(touch)] = touch.location(in: view)
            latestTimestamp = max(latestTimestamp, touch.timestamp)
        }
        evaluate()
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        if state == .possible { reject(reason: "endedBeforeThreshold") }
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        traceRejection(reason: "cancelled")
        state = .cancelled
    }

    override func reset() {
        starts.removeAll(keepingCapacity: true)
        latest.removeAll(keepingCapacity: true)
        latestTimestamp = 0
        didTraceStart = false
        didTraceTerminal = false
        super.reset()
    }

    private func evaluate() {
        guard starts.count == 2, let view else { return }
        let ordered = starts.sorted { $0.value.point.x < $1.value.point.x }
        guard let leftEnd = latest[ordered[0].key], let rightEnd = latest[ordered[1].key] else {
            return
        }
        let leftStart = ordered[0].value
        let rightStart = ordered[1].value
        let sample = StreamingInwardGestureSample(
            leftStart: leftStart.point,
            rightStart: rightStart.point,
            leftEnd: leftEnd,
            rightEnd: rightEnd,
            startDeltaMilliseconds: abs(leftStart.timestamp - rightStart.timestamp) * 1_000,
            durationMilliseconds: (
                latestTimestamp - min(leftStart.timestamp, rightStart.timestamp)
            ) * 1_000,
            viewWidth: view.bounds.width
        )
        if StreamingInwardGesturePolicy.recognizes(sample) {
            didTraceTerminal = true
            state = .recognized
        } else if sample.durationMilliseconds > 450 {
            reject(reason: "timeout", durationMilliseconds: sample.durationMilliseconds)
        } else if abs(leftEnd.y - leftStart.point.y) > 40
            || abs(rightEnd.y - rightStart.point.y) > 40
        {
            reject(reason: "verticalDrift", durationMilliseconds: sample.durationMilliseconds)
        }
    }

    private func validateStartContract() {
        guard starts.count == 2, let view else { return }
        let ordered = starts.sorted { $0.value.point.x < $1.value.point.x }
        let left = ordered[0].value
        let right = ordered[1].value
        if abs(left.timestamp - right.timestamp) * 1_000 > 120 {
            reject(reason: "startTimeDelta")
        } else if left.point.x > view.bounds.width * 0.24
            || right.point.x < view.bounds.width * 0.76
        {
            reject(reason: "startRegion")
        }
    }

    private func reject(reason: String, durationMilliseconds: Double? = nil) {
        traceRejection(reason: reason, durationMilliseconds: durationMilliseconds)
        state = .failed
    }

    private func traceRejection(reason: String, durationMilliseconds: Double? = nil) {
        guard !didTraceTerminal else { return }
        didTraceTerminal = true
        var payload: [String: IOSRuntimeTraceValue] = [
            "reason": .string(reason),
            "pageState": .string("playing"),
        ]
        if let durationMilliseconds {
            payload["durationMs"] = .double(durationMilliseconds)
        }
        trace("streamControlsGestureRejected", payload: payload)
    }

    private func trace(_ event: String, payload: [String: IOSRuntimeTraceValue]) {
        IOSRuntimeTrace.event(
            domain: "ios-streaming",
            event: event,
            payload: payload,
            dimension: .input,
            importance: .debug
        )
    }
}
