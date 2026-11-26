import Foundation
@preconcurrency import WebRTC

struct InputChannelLease: Sendable {
    let identity = UUID()
    let fence = StreamingExecutionFence()
    private let sendOperation: @Sendable (Data) -> Bool
    private let bufferOperation: @Sendable () -> UInt64
    private let readyOperation: @Sendable () -> Bool

    init(channel: RTCDataChannel) {
        let reference = XboxStreamCreatedDataChannel(label: channel.label, channel: channel)
        self.init(
            send: { reference.channel.sendData(RTCDataBuffer(data: $0, isBinary: true)) },
            bufferedAmount: { reference.channel.bufferedAmount },
            isOpen: { reference.channel.readyState == .open })
    }

    init(
        send: @escaping @Sendable (Data) -> Bool,
        bufferedAmount: @escaping @Sendable () -> UInt64 = { 0 },
        isOpen: @escaping @Sendable () -> Bool = { true }
    ) {
        sendOperation = send
        bufferOperation = bufferedAmount
        readyOperation = isOpen
    }

    var bufferedAmount: UInt64 { bufferOperation() }
    var isOpen: Bool { readyOperation() }
    func send(_ data: Data) -> Bool { sendOperation(data) }
}

enum StreamingInputWorkerEvent: Sendable {
    case state(StreamingInputCoreSnapshot)
    case shortcut(MobileShortcutTarget)
    case rumble([IOSStreamRumbleEffect], UInt64)
    case diagnostic(String)
}

/// 所有发送与游戏状态归同一个串行 InputQueue；公开入口只异步投递值。
final class StreamingInputWorker: Sendable {
    let queue = DispatchQueue(label: "xbxrc.ios.streaming.input", qos: .userInteractive)
    let inputFence = StreamingExecutionFence()
    private let feedback: StreamingFrameFeedbackSession?
    private let diagnostics = StreamingDiagnosticsActor()
    private let eventSink: @Sendable (StreamingInputWorkerEvent) -> Void
    private nonisolated(unsafe) var core = StreamInputCore()
    private nonisolated(unsafe) var channel: InputChannelLease?
    private nonisolated(unsafe) var channelRevision: UInt64 = 0
    private nonisolated(unsafe) var timer: DispatchSourceTimer?
    private nonisolated(unsafe) var running = false
    private nonisolated(unsafe) var sequence: UInt32 = 0
    private nonisolated(unsafe) var gate = IOSStreamInputSendGate()
    private nonisolated(unsafe) var nextSendAtMs: Double = 0
    private nonisolated(unsafe) var lastReportAtMs: Double = 0
    private nonisolated(unsafe) var driver: StreamingControllerInputDriver?
    private nonisolated(unsafe) var feedbackScratch: [StreamingPresentedFrameFeedback] = []
    private nonisolated(unsafe) var eventToSendSamples = StreamingBoundedRing<Double>(capacity: 240)
    private nonisolated(unsafe) var sentPackets: UInt64 = 0
    private nonisolated(unsafe) var lastTraceAtMs: Double = 0

    init(
        feedback: StreamingFrameFeedbackSession?,
        eventSink: @escaping @Sendable (StreamingInputWorkerEvent) -> Void
    ) {
        self.feedback = feedback
        self.eventSink = eventSink
        queue.async { [self] in
            driver = StreamingControllerInputDriver(queue: queue) {
                [weak self] sample, continuityLost in
                guard let self else { return }
                if continuityLost {
                    self.core.barrier(
                        epoch: self.inputFence.advance(), suppressed: self.core.isSuppressed,
                        reason: "controllerContinuityLost")
                    self.eventSink(.diagnostic("controllerContinuityLost"))
                }
                self.core.sampleController(sample, at: StreamingFrameFeedbackClock.nowMilliseconds)
                self.wake()
            }
        }
        feedback?.setAvailableHandler { [weak self] in
            self?.queue.async { [weak self] in self?.wake() }
        }
    }

    func attach(_ lease: InputChannelLease) {
        queue.async { [self] in
            channel = lease
            channelRevision = lease.fence.current
        }
    }

    func configure(
        _ configuration: InputConfigurationSnapshot, controller: StreamingControllerLease?
    ) {
        queue.async { [self] in
            core.configure(configuration, at: StreamingFrameFeedbackClock.nowMilliseconds)
            driver?.attach(controller)
            wake()
        }
    }

    func submit(_ batch: InputSampleBatch) {
        queue.async { [self] in
            guard inputFence.accepts(batch.inputEpoch) else { return }
            core.consumeTouch(batch, at: StreamingFrameFeedbackClock.nowMilliseconds)
            wake()
        }
    }

    func barrier(suppressed: Bool, reason: String) {
        let epoch = inputFence.advance()
        queue.async { [self] in
            core.barrier(
                epoch: max(epoch, inputFence.current), suppressed: suppressed, reason: reason)
            eventSink(.diagnostic(reason))
            wake()
        }
    }

    func start() {
        queue.async { [self] in
            guard !running else { return }
            running = true
            let source = DispatchSource.makeTimerSource(queue: queue)
            source.setEventHandler { [weak self] in self?.tick() }
            timer = source
            source.resume()
            wake()
        }
    }

    func sendBootstrap(_ payload: Data) async -> Bool {
        await withCheckedContinuation { continuation in
            queue.async { [self] in
                continuation.resume(returning: send(payload))
            }
        }
    }

    func receiveRumble(_ payload: Data) {
        let epoch = inputFence.current
        queue.async { [self] in
            guard inputFence.accepts(epoch) else { return }
            eventSink(.rumble(IOSStreamRumblePacketDecoder.decode(payload), epoch))
        }
    }

    func stop() {
        let epoch = inputFence.advance()
        queue.async { [self] in
            running = false
            timer?.cancel()
            timer = nil
            driver?.detach()
            core.barrier(epoch: epoch, suppressed: true, reason: "inputStopped")
        }
    }

    func stopAndDrain(policy: IOSStreamInputDrainPolicy) async -> IOSStreamNeutralFrameOutcome {
        stop()
        return await withCheckedContinuation { continuation in
            queue.async { [self] in
                drain(
                    startedAt: StreamingFrameFeedbackClock.nowMilliseconds, policy: policy,
                    continuation: continuation)
            }
        }
    }

    private func drain(
        startedAt: Double, policy: IOSStreamInputDrainPolicy,
        continuation: CheckedContinuation<IOSStreamNeutralFrameOutcome, Never>
    ) {
        guard let channel, channel.isOpen else {
            continuation.resume(returning: .inputUnavailable)
            return
        }
        let now = StreamingFrameFeedbackClock.nowMilliseconds
        if !policy.shouldEnqueueNeutral(
            bufferedAmount: channel.bufferedAmount, elapsedMilliseconds: Int(now - startedAt))
        {
            queue.asyncAfter(deadline: .now() + .milliseconds(policy.pollIntervalMilliseconds)) {
                [self] in
                drain(startedAt: startedAt, policy: policy, continuation: continuation)
            }
            return
        }
        sequence &+= 1
        let sent = send(
            IOSStreamInputPacketEncoder.encodeGamepad(
                sequence: sequence, timestampMilliseconds: now, state: .neutral))
        channel.fence.advance()
        queue.asyncAfter(
            deadline: .now() + .milliseconds(sent ? policy.postNeutralGraceMilliseconds : 0)
        ) {
            continuation.resume(returning: sent ? .sent : .sendFailed)
        }
    }

    private func send(_ payload: Data) -> Bool {
        dispatchPrecondition(condition: .onQueue(queue))
        guard let channel, channel.fence.accepts(channelRevision), channel.isOpen else {
            return false
        }
        return channel.send(payload)
    }

    private func wake() {
        dispatchPrecondition(condition: .onQueue(queue))
        guard running else { return }
        let delay = max(0, nextSendAtMs - StreamingFrameFeedbackClock.nowMilliseconds)
        timer?.schedule(
            deadline: .now() + .nanoseconds(Int(delay * 1_000_000)), leeway: .microseconds(200))
    }

    private func tick() {
        guard running, let channel, channel.fence.accepts(channelRevision) else { return }
        let now = StreamingFrameFeedbackClock.nowMilliseconds
        core.advance(at: now)
        if let action = core.takeShortcutAction() { eventSink(.shortcut(action)) }
        let candidate = core.candidate(at: now)
        let epoch = inputFence.current
        let buffered = channel.bufferedAmount
        let metadataBudget = max(
            0, (Int(IOSStreamInputSendGate.highWatermarkBytes) - Int(clamping: buffered) - 39) / 28)
        feedback?.copyPendingFrames(
            into: &feedbackScratch, maximumCount: metadataBudget, nowMilliseconds: now)
        let frames = feedbackScratch
        if gate.shouldSend(
            state: candidate.state, bufferedAmount: buffered, nowMilliseconds: now,
            force: candidate.barrier, hasFrameFeedback: !frames.isEmpty), inputFence.accepts(epoch)
        {
            let nextSequence = sequence &+ 1
            var payload = IOSStreamInputPacketEncoder.encodeGamepad(
                sequence: nextSequence, timestampMilliseconds: now, state: candidate.state)
            var encodedFeedback = false
            if !frames.isEmpty,
                let encoded = try? streamInputFrameFeedbackPayload(
                    gamepadPayload: payload, frames: frames.map(\.metadata), timestampMs: now,
                    preventResolutionDrops: feedback?.preventResolutionDrops ?? false)
            {
                payload = encoded
                encodedFeedback = true
            }
            if inputFence.accepts(epoch), send(payload) {
                let acceptedAt = StreamingFrameFeedbackClock.nowMilliseconds
                if !candidate.barrier, candidate.digital || gate.lastSentState != candidate.state {
                    eventToSendSamples.append(max(0, acceptedAt - candidate.collectedAtMs))
                }
                sequence = nextSequence
                sentPackets &+= 1
                gate.markSent(state: candidate.state, at: acceptedAt)
                core.accepted(candidate)
                if encodedFeedback { feedback?.markSent(frames, wirePayload: payload) }
            }
            nextSendAtMs = StreamingFrameFeedbackClock.nowMilliseconds + 8
        }
        if now - lastReportAtMs >= 250 {
            lastReportAtMs = now
            eventSink(.state(core.snapshot))
        }
        if now - lastTraceAtMs >= 1_000 {
            lastTraceAtMs = now
            let snapshot = core.snapshot
            let samples = eventToSendSamples.values()
            let count = sentPackets
            let feedback = feedback?.snapshot()
            Task(priority: .utility) { [diagnostics] in
                await diagnostics.recordInput(
                    snapshot, samples: samples, sentPackets: count, feedback: feedback)
            }
        }
        let active =
            candidate.barrier || core.needsSamplingTicks || core.snapshot.pendingEdges > 0
            || gate.blocked || core.snapshot.state != .neutral || !frames.isEmpty
        timer?.schedule(
            deadline: .now() + .milliseconds(active ? 8 : 250), leeway: .microseconds(200))
    }
}
