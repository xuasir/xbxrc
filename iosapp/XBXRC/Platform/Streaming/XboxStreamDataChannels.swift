import CoreHaptics
import Foundation
import GameController
@preconcurrency import WebRTC

struct XboxStreamCreatedDataChannel: @unchecked Sendable {
    let label: String
    let channel: RTCDataChannel
}

@MainActor
final class XboxStreamDataChannels: NSObject {
    private static let gamepadAddedDelay = Duration.milliseconds(500)
    private static let inputPollInterval = Duration.milliseconds(8)
    private static let bootstrapRetryDelay = Duration.milliseconds(100)

    private let inputSession: StreamInputSession
    private let frameFeedback: StreamingFrameFeedbackSession?
    private var lastFeedbackTraceAtMilliseconds: Double = 0
    private let eventSink: @MainActor @Sendable (String, String) -> Void
    private let failureSink: @MainActor @Sendable (String) -> Void
    private let postHandshakePayloads: [String]
    private let controlBootstrapPayloads: [String]
    private var channels: [String: RTCDataChannel] = [:]
    private var stateMachine: StreamingDataChannelStateMachine
    private var delayedGamepadAddedTask: Task<Void, Never>?
    private var bootstrapRetryTask: Task<Void, Never>?
    private var inputTask: Task<Void, Never>?
    private var initialGamepadAnnouncementPending = false
    private var gamepadConnected = false
    private weak var announcedController: GCController?
    private var locallyClosing = false
    private var inputAndHapticsStopped = false
    private var inputSequence: UInt32 = 0
    private var inputGate = IOSStreamInputSendGate()
    private var hapticEngines: [IOSStreamHapticsRoute: CHHapticEngine] = [:]
    private var hapticPlayers: [IOSStreamHapticsRoute: CHHapticPatternPlayer] = [:]
    private weak var hapticsController: GCController?

    init(
        inputSession: StreamInputSession = StreamInputSession(),
        frameFeedback: StreamingFrameFeedbackSession? = nil,
        targetVideoWidth: Int = 1_920,
        targetVideoHeight: Int = 1_080,
        eventSink: @escaping @MainActor @Sendable (String, String) -> Void,
        failureSink: @escaping @MainActor @Sendable (String) -> Void = { _ in }
    ) {
        self.inputSession = inputSession
        self.frameFeedback = frameFeedback
        self.eventSink = eventSink
        self.failureSink = failureSink
        postHandshakePayloads =
            streamPostHandshakePayloads(
                width: UInt32(clamping: max(targetVideoWidth, 1)),
                height: UInt32(clamping: max(targetVideoHeight, 1))
            ) + streamTouchCapabilityPayloads(enabled: true)
        controlBootstrapPayloads = streamControlBootstrapPayloads()
        stateMachine = StreamingDataChannelStateMachine(
            postHandshakeCount: postHandshakePayloads.count,
            controlBootstrapCount: controlBootstrapPayloads.count
        )
    }

    func requestVideoKeyframe() -> Bool {
        guard !locallyClosing, let channel = channels["control"], channel.readyState == .open else {
            return false
        }
        return sendText(
            "{\"message\":\"videoKeyframeRequested\",\"ifrRequested\":true}", on: channel)
    }

    func createChannels(on peerConnection: RTCPeerConnection) {
        locallyClosing = false
        inputAndHapticsStopped = false
        for profile in streamDataChannelProfiles() {
            let configuration = RTCDataChannelConfiguration()
            configuration.isOrdered = profile.ordered
            configuration.`protocol` = profile.protocolName
            guard
                let channel = peerConnection.dataChannel(
                    forLabel: profile.label,
                    configuration: configuration
                )
            else {
                eventSink(profile.label, "createFailed")
                failureSink(StreamingTerminalReason.runtimeFailure.code)
                continue
            }
            channel.delegate = self
            channels[profile.label] = channel
            eventSink(profile.label, "created")
        }
    }

    func attachCreatedChannels(
        _ createdChannels: [XboxStreamCreatedDataChannel],
        failedLabels: [String]
    ) {
        locallyClosing = false
        inputAndHapticsStopped = false
        for label in failedLabels {
            eventSink(label, "createFailed")
            failureSink(StreamingTerminalReason.runtimeFailure.code)
        }
        for created in createdChannels {
            created.channel.delegate = self
            channels[created.label] = created.channel
            eventSink(created.label, "created")
        }
    }

    func stopInputAndHaptics() {
        guard !inputAndHapticsStopped else { return }
        inputAndHapticsStopped = true
        frameFeedback?.invalidate()
        delayedGamepadAddedTask?.cancel()
        delayedGamepadAddedTask = nil
        initialGamepadAnnouncementPending = false
        inputTask?.cancel()
        inputTask = nil
        eventSink("input", "inputStopped")
        resetHaptics()
        eventSink("input", "hapticsCleanupCompleted")
    }

    func closeAfterInputDrain(
        policy: IOSStreamInputDrainPolicy = .standard
    ) async {
        locallyClosing = true
        bootstrapRetryTask?.cancel()
        bootstrapRetryTask = nil
        stopInputAndHaptics()
        let outcome = await drainAndSendNeutralFrame(policy: policy)
        eventSink("input", "neutralFrame\(outcome.rawValue.capitalized)")
        for channel in channels.values {
            channel.delegate = nil
            channel.close()
        }
        channels.removeAll()
        eventSink("all", "dataChannelsClosed")
        gamepadConnected = false
        inputSequence = 0
        inputGate.reset()
        stateMachine = StreamingDataChannelStateMachine(
            postHandshakeCount: postHandshakePayloads.count,
            controlBootstrapCount: controlBootstrapPayloads.count
        )
    }

    func debugSnapshot() -> StreamingDataChannelDebugSnapshot {
        let snapshot = stateMachine.snapshot
        return StreamingDataChannelDebugSnapshot(
            readyStates: channels.mapValues { Self.readyStateName($0.readyState) },
            phases: Dictionary(
                uniqueKeysWithValues: snapshot.phases.map { ($0.key.rawValue, $0.value.rawValue) }
            ),
            handshakeAcknowledged: snapshot.handshakeAcknowledged,
            controlReady: snapshot.controlReady,
            inputStarted: snapshot.inputStarted,
            terminalReason: snapshot.terminalReason?.code
        )
    }

    private func drainAndSendNeutralFrame(
        policy: IOSStreamInputDrainPolicy
    ) async -> IOSStreamNeutralFrameOutcome {
        guard let input = channels["input"], input.readyState == .open else {
            return .inputUnavailable
        }
        let clock = ContinuousClock()
        let startedAt = clock.now
        while input.readyState == .open {
            let elapsed = startedAt.duration(to: clock.now)
            let elapsedMilliseconds = Int(
                Double(elapsed.components.seconds) * 1_000
                    + Double(elapsed.components.attoseconds) / 1_000_000_000_000_000
            )
            if policy.shouldEnqueueNeutral(
                bufferedAmount: input.bufferedAmount,
                elapsedMilliseconds: elapsedMilliseconds
            ) {
                break
            }
            do {
                try await Task.sleep(for: .milliseconds(policy.pollIntervalMilliseconds))
            } catch {
                break
            }
        }
        guard input.readyState == .open else { return .inputUnavailable }
        let now = ProcessInfo.processInfo.systemUptime * 1_000
        guard
            inputGate.shouldSend(
                state: .neutral,
                bufferedAmount: input.bufferedAmount,
                nowMilliseconds: now,
                force: true
            )
        else { return .sendFailed }
        inputSequence &+= 1
        let frame = IOSStreamInputPacketEncoder.encodeGamepad(
            sequence: inputSequence,
            timestampMilliseconds: now,
            state: .neutral
        )
        if sendBinary(frame, on: input) {
            inputGate.markSent(state: .neutral, at: now)
            eventSink("input", "neutralFrameSent")
            try? await Task.sleep(for: .milliseconds(policy.postNeutralGraceMilliseconds))
            return .sent
        }
        eventSink("input", "neutralFrameSendFailed")
        return .sendFailed
    }

    private func scheduleInitialGamepadAnnouncement(on channel: RTCDataChannel) {
        delayedGamepadAddedTask?.cancel()
        initialGamepadAnnouncementPending = true
        delayedGamepadAddedTask = Task { @MainActor [weak self, weak channel] in
            do {
                try await Task.sleep(for: Self.gamepadAddedDelay)
                guard let self else { return }
                self.initialGamepadAnnouncementPending = false
                guard let channel, channel.readyState == .open else { return }
                let added = self.currentGamepad() != nil || self.inputSession.showsTouchController
                let sent = self.sendText(
                    streamControlGamepadChangedPayload(added: added),
                    on: channel
                )
                self.eventSink("control", sent ? "gamepadAddedSent" : "gamepadAddedFailed")
                if sent {
                    self.gamepadConnected = added
                    self.announcedController = self.currentGamepadController()
                }
            } catch is CancellationError {
                self?.initialGamepadAnnouncementPending = false
                return
            } catch {
                self?.initialGamepadAnnouncementPending = false
                self?.eventSink("control", "gamepadAddedFailed")
            }
        }
    }

    private func handleMessagePayload(_ data: Data, channel: RTCDataChannel) {
        if !locallyClosing, data.count <= 128 * 1024,
            let payload = String(data: data, encoding: .utf8),
            let command = parseStreamTouchLayoutCommand(payload: payload)
        {
            inputSession.onTouchLayoutCommand?(command)
            eventSink("message", "touchLayoutCommandReceived")
        }
        let wasAcknowledged = stateMachine.snapshot.handshakeAcknowledged
        if let reason = stateMachine.receiveMessage(data) {
            reportTerminal(reason, channel: channel.label)
            return
        }
        if !wasAcknowledged, stateMachine.snapshot.handshakeAcknowledged {
            eventSink("message", "handshakeAcked")
        }
        processBootstrapActions()
    }

    private func processBootstrapActions() {
        bootstrapRetryTask?.cancel()
        bootstrapRetryTask = nil
        while let action = stateMachine.nextAction() {
            let succeeded: Bool
            switch action {
            case .sendMessageHandshake:
                succeeded =
                    channel(.message).map {
                        sendText(streamMessageHandshakePayload(), on: $0)
                    } ?? false
                if succeeded { eventSink("message", "handshakeSent") }
            case .sendPostHandshake(let index):
                succeeded =
                    channel(.message).map {
                        sendText(postHandshakePayloads[index], on: $0)
                    } ?? false
                if succeeded, index + 1 == postHandshakePayloads.count {
                    eventSink("message", "postHandshakeCompleted")
                }
            case .sendControlBootstrap(let stage, let index):
                succeeded =
                    channel(.control).map {
                        sendText(controlBootstrapPayloads[index], on: $0)
                    } ?? false
                if succeeded, index + 1 == controlBootstrapPayloads.count {
                    let event =
                        switch stage {
                        case .preHandshake: "bootstrapPreHandshakeSent"
                        case .postHandshake: "bootstrapSent"
                        }
                    eventSink("control", event)
                }
            case .sendInputMetadata(let stage):
                succeeded =
                    channel(.input).map {
                        sendBinary(Data(streamInputMetadataBootstrapPayload()), on: $0)
                    } ?? false
                if succeeded {
                    let event =
                        switch stage {
                        case .preHandshake: "metadataPreHandshakeSent"
                        case .postHandshake: "metadataBootstrapSent"
                        }
                    eventSink("input", event)
                }
            case .announceControlReady:
                eventSink("control", "ready")
                succeeded = true
            case .scheduleGamepadAnnouncement:
                if let control = channel(.control) {
                    scheduleInitialGamepadAnnouncement(on: control)
                    succeeded = true
                } else {
                    succeeded = false
                }
            case .startInput:
                startInputLoopIfReady()
                succeeded = inputTask != nil
            }
            if succeeded {
                stateMachine.actionDidSucceed(action)
                continue
            }
            stateMachine.actionDidFail(action)
            eventSink(actionChannel(action).rawValue, "bootstrapActionFailed")
            scheduleBootstrapRetry()
            return
        }
    }

    private func scheduleBootstrapRetry() {
        guard bootstrapRetryTask == nil, stateMachine.terminalReason == nil else { return }
        bootstrapRetryTask = Task { @MainActor [weak self] in
            do {
                try await Task.sleep(for: Self.bootstrapRetryDelay)
                guard let self else { return }
                self.bootstrapRetryTask = nil
                self.processBootstrapActions()
            } catch {
                self?.bootstrapRetryTask = nil
            }
        }
    }

    private func channel(_ label: StreamingDataChannelLabel) -> RTCDataChannel? {
        guard let channel = channels[label.rawValue], channel.readyState == .open else {
            return nil
        }
        return channel
    }

    private func actionChannel(
        _ action: StreamingDataChannelBootstrapAction
    ) -> StreamingDataChannelLabel {
        switch action {
        case .sendMessageHandshake, .sendPostHandshake: .message
        case .sendControlBootstrap, .announceControlReady, .scheduleGamepadAnnouncement: .control
        case .sendInputMetadata, .startInput: .input
        }
    }

    private func reportTerminal(_ reason: StreamingTerminalReason, channel: String) {
        bootstrapRetryTask?.cancel()
        bootstrapRetryTask = nil
        stopInputAndHaptics()
        eventSink(channel, "terminal:\(reason.code)")
        failureSink(reason.code)
    }

    @discardableResult
    private func sendText(_ payload: String, on channel: RTCDataChannel) -> Bool {
        let data = Data(payload.utf8)
        let sent = channel.sendData(RTCDataBuffer(data: data, isBinary: false))
        eventSink(channel.label, sent ? "messageSent" : "messageSendFailed")
        return sent
    }

    @discardableResult
    private func sendBinary(_ payload: Data, on channel: RTCDataChannel) -> Bool {
        let sent = channel.sendData(RTCDataBuffer(data: payload, isBinary: true))
        eventSink(channel.label, sent ? "binarySent" : "binarySendFailed")
        return sent
    }

    private func startInputLoopIfReady() {
        let snapshot = stateMachine.snapshot
        guard inputTask == nil, snapshot.handshakeAcknowledged,
            snapshot.controlReady,
            snapshot.postHandshakeInputMetadataSent
        else { return }
        inputTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                guard let self,
                    let input = self.channels["input"], input.readyState == .open
                else { return }
                let controller = self.currentGamepadController()
                let connected =
                    controller?.extendedGamepad != nil || self.inputSession.showsTouchController
                if !self.hapticEngines.isEmpty,
                    controller == nil || self.hapticsController !== controller
                {
                    self.resetHaptics()
                }
                if !self.initialGamepadAnnouncementPending,
                    connected != self.gamepadConnected || controller !== self.announcedController,
                    let control = self.channels["control"], control.readyState == .open,
                    self.sendText(
                        streamControlGamepadChangedPayload(added: connected),
                        on: control
                    )
                {
                    self.gamepadConnected = connected
                    self.announcedController = controller
                    self.eventSink("control", connected ? "gamepadAddedSent" : "gamepadRemovedSent")
                }
                let now =
                    self.frameFeedback == nil
                    ? ProcessInfo.processInfo.systemUptime * 1_000
                    : StreamingFrameFeedbackClock.nowMilliseconds
                let state = self.inputSession.outputState(now: now)
                if self.inputSession.isSuppressed || !self.inputSession.preferences.vibrationEnabled
                {
                    if !self.hapticPlayers.isEmpty { self.resetHaptics() }
                }
                let wasBlocked = self.inputGate.blocked
                // Metadata 在原包中消费剩余预算，游戏按键与中立释放继续拥有发送余量。
                let metadataBudget = max(
                    0,
                    Int(
                        (Double(IOSStreamInputSendGate.highWatermarkBytes)
                            - Double(input.bufferedAmount) - 39) / 28))
                let feedbackFrames =
                    self.frameFeedback?.pendingFrames(
                        maximumCount: metadataBudget, nowMilliseconds: now) ?? []
                if self.inputGate.shouldSend(
                    state: state,
                    bufferedAmount: input.bufferedAmount,
                    nowMilliseconds: now,
                    hasFrameFeedback: !feedbackFrames.isEmpty
                ) {
                    self.inputSequence &+= 1
                    var frame = IOSStreamInputPacketEncoder.encodeGamepad(
                        sequence: self.inputSequence,
                        timestampMilliseconds: now,
                        state: state
                    )
                    var encodedFeedback = false
                    if !feedbackFrames.isEmpty {
                        do {
                            frame = try streamInputFrameFeedbackPayload(
                                gamepadPayload: frame, frames: feedbackFrames.map(\.metadata),
                                timestampMs: now,
                                preventResolutionDrops: self.frameFeedback?.preventResolutionDrops
                                    ?? false)
                            encodedFeedback = true
                        } catch {
                            self.eventSink("input", "frameFeedbackInvalid")
                        }
                    }
                    if self.sendBinary(frame, on: input) {
                        self.inputGate.markSent(state: state, at: now)
                        if encodedFeedback {
                            self.frameFeedback?.markSent(feedbackFrames, wirePayload: frame)
                            if now - self.lastFeedbackTraceAtMilliseconds >= 1_000,
                                let snapshot = self.frameFeedback?.snapshot()
                            {
                                self.lastFeedbackTraceAtMilliseconds = now
                                IOSRuntimeTrace.snapshot(
                                    domain: "ios-streaming", event: "frameFeedback",
                                    payload: snapshot.tracePayload(), dimension: .presentation,
                                    importance: .debug)
                            }
                        }
                        self.eventSink(
                            "input",
                            state == .neutral ? "neutralFrameSent" : "frameSent"
                        )
                    }
                    if wasBlocked { self.eventSink("input", "backpressureReleased") }
                } else if self.inputGate.blocked, !wasBlocked {
                    self.eventSink("input", "backpressureDrop")
                }
                do {
                    try await Task.sleep(for: Self.inputPollInterval)
                } catch is CancellationError {
                    return
                } catch {
                    return
                }
            }
        }
    }

    private func currentGamepadController() -> GCController? {
        inputSession.currentController
    }

    private func currentGamepad() -> GCExtendedGamepad? {
        currentGamepadController()?.extendedGamepad
    }

    private func handleInputPayload(_ data: Data) {
        guard !inputSession.isSuppressed, inputSession.preferences.vibrationEnabled else {
            resetHaptics()
            return
        }
        for effect in IOSStreamRumblePacketDecoder.decode(data) {
            eventSink("input", "rumbleParsed")
            if effect.isStop {
                applyHapticsTransition(
                    IOSStreamHapticsTransitionPlanner.plan(
                        activeRoutes: Set(hapticPlayers.keys),
                        nextPulses: [],
                        stopAll: true
                    ),
                    controller: nil
                )
                eventSink("input", "rumbleStopped")
                continue
            }
            guard let controller = gamepadController(for: effect.gamepadIndex) else {
                applyHapticsTransition(
                    IOSStreamHapticsTransitionPlanner.plan(
                        activeRoutes: Set(hapticPlayers.keys),
                        nextPulses: [],
                        stopAll: true
                    ),
                    controller: nil
                )
                eventSink("input", "rumbleTargetUnavailable")
                continue
            }
            let pulses = IOSStreamHapticsRouter.pulses(
                for: inputSession.preferences.vibrationEffect(effect),
                supportedRoutes: supportedHapticsRoutes(for: controller)
            )
            let transition = IOSStreamHapticsTransitionPlanner.plan(
                activeRoutes: Set(hapticPlayers.keys),
                nextPulses: pulses
            )
            applyHapticsTransition(transition, controller: controller)
            guard !pulses.isEmpty else {
                eventSink("input", "hapticsUnsupported")
                continue
            }
            var applied = true
            for pulse in transition.pulsesToPlay {
                if !playHaptics(pulse, on: controller) { applied = false }
            }
            if applied {
                eventSink("input", "hapticsApplied")
                if pulses.contains(where: {
                    [.handles, .triggers, .defaultLocality].contains($0.route)
                }) {
                    eventSink("input", "hapticsDegraded")
                }
            }
        }
    }

    private func applyHapticsTransition(
        _ transition: IOSStreamHapticsTransition,
        controller _: GCController?
    ) {
        stopHaptics(routes: transition.routesToStop)
    }

    func gamepadController(for index: UInt8?) -> GCController? {
        // 当前会话公告一个逻辑slot 0，输入和四路震动共同归属所选设备。
        guard index == nil || index == 0 else { return nil }
        return inputSession.currentController
    }

    private func supportedHapticsRoutes(for controller: GCController) -> Set<IOSStreamHapticsRoute>
    {
        guard let haptics = controller.haptics else { return [] }
        var routes: Set<IOSStreamHapticsRoute> = [.defaultLocality]
        let localities = haptics.supportedLocalities
        if localities.contains(.leftHandle) { routes.insert(.leftHandle) }
        if localities.contains(.rightHandle) { routes.insert(.rightHandle) }
        if localities.contains(.leftTrigger) { routes.insert(.leftTrigger) }
        if localities.contains(.rightTrigger) { routes.insert(.rightTrigger) }
        if localities.contains(.handles) { routes.insert(.handles) }
        if localities.contains(.triggers) { routes.insert(.triggers) }
        return routes
    }

    @discardableResult
    private func playHaptics(_ pulse: IOSStreamHapticsPulse, on controller: GCController) -> Bool {
        guard let haptics = controller.haptics else { return false }
        if let hapticsController, hapticsController !== controller {
            resetHaptics()
        }
        hapticsController = controller
        let locality: GCHapticsLocality =
            switch pulse.route {
            case .leftHandle: .leftHandle
            case .rightHandle: .rightHandle
            case .leftTrigger: .leftTrigger
            case .rightTrigger: .rightTrigger
            case .handles: .handles
            case .triggers: .triggers
            case .defaultLocality: .default
            }
        guard
            let engine = hapticEngines[pulse.route] ?? haptics.createEngine(withLocality: locality)
        else {
            eventSink("input", "rumbleFailed")
            return false
        }
        hapticEngines[pulse.route] = engine
        do {
            try engine.start()
            let events = pulse.startTimesMilliseconds.map { startTime in
                CHHapticEvent(
                    eventType: .hapticContinuous,
                    parameters: [
                        CHHapticEventParameter(
                            parameterID: .hapticIntensity,
                            value: pulse.intensity
                                * Float(inputSession.preferences.vibrationIntensity)),
                        CHHapticEventParameter(
                            parameterID: .hapticSharpness, value: pulse.sharpness),
                    ],
                    relativeTime: Double(startTime) / 1_000,
                    duration: Double(pulse.durationMilliseconds) / 1_000
                )
            }
            let pattern = try CHHapticPattern(events: events, parameters: [])
            try? hapticPlayers[pulse.route]?.stop(atTime: 0)
            let player = try engine.makePlayer(with: pattern)
            hapticPlayers[pulse.route] = player
            try player.start(atTime: 0)
            return true
        } catch {
            eventSink("input", "rumbleFailed")
            return false
        }
    }

    private func stopHaptics() {
        stopHaptics(routes: Set(hapticPlayers.keys))
        eventSink("input", "hapticsStopped")
    }

    private func stopHaptics(routes: Set<IOSStreamHapticsRoute>) {
        for route in routes {
            try? hapticPlayers.removeValue(forKey: route)?.stop(atTime: 0)
        }
    }

    private func resetHaptics() {
        stopHaptics()
        for engine in hapticEngines.values { engine.stop(completionHandler: nil) }
        hapticEngines.removeAll()
        hapticsController = nil
    }

    private static func readyStateName(_ state: RTCDataChannelState) -> String {
        switch state {
        case .connecting: "connecting"
        case .open: "open"
        case .closing: "closing"
        case .closed: "closed"
        @unknown default: "unknown"
        }
    }
}

extension XboxStreamDataChannels: RTCDataChannelDelegate {
    nonisolated func dataChannelDidChangeState(_ dataChannel: RTCDataChannel) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            self.eventSink(dataChannel.label, String(describing: dataChannel.readyState))
            guard let label = StreamingDataChannelLabel(wireLabel: dataChannel.label) else {
                return
            }
            if dataChannel.readyState == .closed {
                guard !self.locallyClosing else { return }
                if label == .input { self.stopInputAndHaptics() }
                if let reason = self.stateMachine.channelDidClose(label) {
                    self.reportTerminal(reason, channel: dataChannel.label)
                }
                return
            }
            guard dataChannel.readyState == .open else { return }
            self.stateMachine.channelDidOpen(label)
            self.processBootstrapActions()
        }
    }

    nonisolated func dataChannel(
        _ dataChannel: RTCDataChannel,
        didReceiveMessageWith buffer: RTCDataBuffer
    ) {
        let data = buffer.data
        Task { @MainActor [weak self] in
            guard let self else { return }
            self.eventSink(dataChannel.label, "messageReceived")
            if dataChannel.label == "message", !buffer.isBinary {
                self.handleMessagePayload(data, channel: dataChannel)
            } else if dataChannel.label == "input", buffer.isBinary {
                self.handleInputPayload(data)
            }
        }
    }
}
