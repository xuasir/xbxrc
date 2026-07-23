#if canImport(WebRTC)
import CoreHaptics
import Foundation
import GameController
@preconcurrency import WebRTC

@MainActor
final class XboxStreamDataChannels: NSObject {
    private static let gamepadAddedDelay = Duration.milliseconds(500)
    private static let inputPollInterval = Duration.milliseconds(8)
    private static let bootstrapRetryDelay = Duration.milliseconds(100)

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
    private var locallyClosing = false
    private var inputAndHapticsStopped = false
    private var inputSequence: UInt32 = 0
    private var inputGate = IOSStreamInputSendGate()
    private var hapticEngines: [IOSStreamHapticsRoute: CHHapticEngine] = [:]
    private var hapticPlayers: [IOSStreamHapticsRoute: CHHapticPatternPlayer] = [:]
    private weak var hapticsController: GCController?

    init(
        targetVideoWidth: Int = 1_920,
        targetVideoHeight: Int = 1_080,
        eventSink: @escaping @MainActor @Sendable (String, String) -> Void,
        failureSink: @escaping @MainActor @Sendable (String) -> Void = { _ in }
    ) {
        self.eventSink = eventSink
        self.failureSink = failureSink
        postHandshakePayloads = streamPostHandshakePayloads(
            width: UInt32(clamping: max(targetVideoWidth, 1)),
            height: UInt32(clamping: max(targetVideoHeight, 1))
        )
        controlBootstrapPayloads = streamControlBootstrapPayloads()
        stateMachine = StreamingDataChannelStateMachine(
            postHandshakeCount: postHandshakePayloads.count,
            controlBootstrapCount: controlBootstrapPayloads.count
        )
    }

    func createChannels(on peerConnection: RTCPeerConnection) {
        locallyClosing = false
        inputAndHapticsStopped = false
        for profile in streamDataChannelProfiles() {
            let configuration = RTCDataChannelConfiguration()
            configuration.isOrdered = profile.ordered
            configuration.`protocol` = profile.protocolName
            guard let channel = peerConnection.dataChannel(
                forLabel: profile.label,
                configuration: configuration
            ) else {
                eventSink(profile.label, "createFailed")
                failureSink(StreamingTerminalReason.runtimeFailure.code)
                continue
            }
            channel.delegate = self
            channels[profile.label] = channel
            eventSink(profile.label, "created")
        }
    }

    func stopInputAndHaptics() {
        guard !inputAndHapticsStopped else { return }
        inputAndHapticsStopped = true
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
        guard inputGate.shouldSend(
            state: .neutral,
            bufferedAmount: input.bufferedAmount,
            nowMilliseconds: now,
            force: true
        ) else { return .sendFailed }
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
                let added = self.currentGamepad() != nil
                let sent = self.sendText(
                    streamControlGamepadChangedPayload(added: added),
                    on: channel
                )
                self.eventSink("control", sent ? "gamepadAddedSent" : "gamepadAddedFailed")
                if sent { self.gamepadConnected = added }
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
                succeeded = channel(.message).map {
                    sendText(streamMessageHandshakePayload(), on: $0)
                } ?? false
                if succeeded { eventSink("message", "handshakeSent") }
            case let .sendPostHandshake(index):
                succeeded = channel(.message).map {
                    sendText(postHandshakePayloads[index], on: $0)
                } ?? false
                if succeeded, index + 1 == postHandshakePayloads.count {
                    eventSink("message", "postHandshakeCompleted")
                }
            case let .sendControlBootstrap(stage, index):
                succeeded = channel(.control).map {
                    sendText(controlBootstrapPayloads[index], on: $0)
                } ?? false
                if succeeded, index + 1 == controlBootstrapPayloads.count {
                    let event = switch stage {
                    case .preHandshake: "bootstrapPreHandshakeSent"
                    case .postHandshake: "bootstrapSent"
                    }
                    eventSink("control", event)
                }
            case let .sendInputMetadata(stage):
                succeeded = channel(.input).map {
                    sendBinary(Data(streamInputMetadataBootstrapPayload()), on: $0)
                } ?? false
                if succeeded {
                    let event = switch stage {
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
        guard let channel = channels[label.rawValue], channel.readyState == .open else { return nil }
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
              snapshot.postHandshakeInputMetadataSent else { return }
        inputTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                guard let self,
                      let input = self.channels["input"], input.readyState == .open else { return }
                let controller = self.currentGamepadController()
                let connected = controller?.extendedGamepad != nil
                if !connected, self.hapticsController != nil {
                    self.resetHaptics()
                }
                if !self.initialGamepadAnnouncementPending,
                   connected != self.gamepadConnected,
                   let control = self.channels["control"], control.readyState == .open,
                   self.sendText(
                       streamControlGamepadChangedPayload(added: connected),
                       on: control
                   )
                {
                    self.gamepadConnected = connected
                    self.eventSink("control", connected ? "gamepadAddedSent" : "gamepadRemovedSent")
                }
                let now = ProcessInfo.processInfo.systemUptime * 1_000
                let state = self.gamepadState(controller?.extendedGamepad)
                let wasBlocked = self.inputGate.blocked
                if self.inputGate.shouldSend(
                    state: state,
                    bufferedAmount: input.bufferedAmount,
                    nowMilliseconds: now
                ) {
                    self.inputSequence &+= 1
                    let frame = IOSStreamInputPacketEncoder.encodeGamepad(
                        sequence: self.inputSequence,
                        timestampMilliseconds: now,
                        state: state
                    )
                    if self.sendBinary(frame, on: input) {
                        self.inputGate.markSent(state: state, at: now)
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
        if let current = GCController.current, current.extendedGamepad != nil {
            return current
        }
        return GCController.controllers().first { $0.extendedGamepad != nil }
    }

    private func currentGamepad() -> GCExtendedGamepad? {
        currentGamepadController()?.extendedGamepad
    }

    private func gamepadState(_ gamepad: GCExtendedGamepad?) -> IOSStreamGamepadState {
        guard let gamepad else { return .neutral }
        var buttonMask: UInt16 = 0
        if gamepad.buttonHome?.isPressed == true { buttonMask |= 2 }
        if gamepad.buttonMenu.isPressed { buttonMask |= 4 }
        if gamepad.buttonOptions?.isPressed == true { buttonMask |= 8 }
        if gamepad.buttonA.isPressed { buttonMask |= 16 }
        if gamepad.buttonB.isPressed { buttonMask |= 32 }
        if gamepad.buttonX.isPressed { buttonMask |= 64 }
        if gamepad.buttonY.isPressed { buttonMask |= 128 }
        if gamepad.dpad.up.isPressed { buttonMask |= 256 }
        if gamepad.dpad.down.isPressed { buttonMask |= 512 }
        if gamepad.dpad.left.isPressed { buttonMask |= 1024 }
        if gamepad.dpad.right.isPressed { buttonMask |= 2048 }
        if gamepad.leftShoulder.isPressed { buttonMask |= 4096 }
        if gamepad.rightShoulder.isPressed { buttonMask |= 8192 }
        if gamepad.leftThumbstickButton?.isPressed == true { buttonMask |= 16384 }
        if gamepad.rightThumbstickButton?.isPressed == true { buttonMask |= 32768 }
        return IOSStreamGamepadState(
            buttonMask: buttonMask,
            leftThumbX: gamepad.leftThumbstick.xAxis.value,
            leftThumbY: gamepad.leftThumbstick.yAxis.value,
            rightThumbX: gamepad.rightThumbstick.xAxis.value,
            rightThumbY: gamepad.rightThumbstick.yAxis.value,
            leftTrigger: gamepad.leftTrigger.value,
            rightTrigger: gamepad.rightTrigger.value
        )
    }

    private func handleInputPayload(_ data: Data) {
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
                for: effect,
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

    private func gamepadController(for index: UInt8?) -> GCController? {
        guard let index else { return currentGamepadController() }
        if index == 0, let current = currentGamepadController() { return current }
        let controllers = GCController.controllers().filter { $0.extendedGamepad != nil }
        return Int(index) < controllers.count ? controllers[Int(index)] : nil
    }

    private func supportedHapticsRoutes(for controller: GCController) -> Set<IOSStreamHapticsRoute> {
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
        let locality: GCHapticsLocality = switch pulse.route {
        case .leftHandle: .leftHandle
        case .rightHandle: .rightHandle
        case .leftTrigger: .leftTrigger
        case .rightTrigger: .rightTrigger
        case .handles: .handles
        case .triggers: .triggers
        case .defaultLocality: .default
        }
        guard let engine = hapticEngines[pulse.route] ?? haptics.createEngine(withLocality: locality) else {
            eventSink("input", "rumbleFailed")
            return false
        }
        hapticEngines[pulse.route] = engine
        do {
            try engine.start()
            let pattern = try CHHapticPattern(events: [
                CHHapticEvent(
                    eventType: .hapticContinuous,
                    parameters: [
                        CHHapticEventParameter(parameterID: .hapticIntensity, value: pulse.intensity),
                        CHHapticEventParameter(parameterID: .hapticSharpness, value: pulse.sharpness),
                    ],
                    relativeTime: 0,
                    duration: max(0.01, Double(pulse.durationMilliseconds) / 1_000)
                ),
            ], parameters: [])
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
#endif
