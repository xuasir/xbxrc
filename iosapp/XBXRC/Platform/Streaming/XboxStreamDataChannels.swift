import CoreHaptics
import Foundation
import GameController
@preconcurrency import WebRTC

struct XboxStreamCreatedDataChannel: @unchecked Sendable {
    let label: String
    let channel: RTCDataChannel
}

private final class StreamingDataChannelSnapshotStore: Sendable {
    private let lock = NSLock()
    private nonisolated(unsafe) var snapshot = StreamingDataChannelDebugSnapshot(
        readyStates: [:], phases: [:], handshakeAcknowledged: false, controlReady: false,
        inputStarted: false, terminalReason: nil)
    var value: StreamingDataChannelDebugSnapshot { lock.withLock { snapshot } }
    func update(_ value: StreamingDataChannelDebugSnapshot) { lock.withLock { snapshot = value } }
}

private final class StreamingInputEventRelay: Sendable {
    private let lock = NSLock()
    private nonisolated(unsafe) var sink:
        (@MainActor @Sendable (StreamingInputWorkerEvent) -> Void)?
    func bind(_ value: @escaping @MainActor @Sendable (StreamingInputWorkerEvent) -> Void) {
        lock.withLock { sink = value }
    }
    func send(_ event: StreamingInputWorkerEvent) {
        let handler = lock.withLock { sink }
        Task { @MainActor in handler?(event) }
    }
}

@MainActor
final class XboxStreamDataChannels: NSObject {
    private static let gamepadAddedDelay = Duration.milliseconds(500)
    private static let bootstrapRetryDelay = Duration.milliseconds(100)

    private nonisolated let snapshotStore = StreamingDataChannelSnapshotStore()
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
    private var inputStarted = false
    private var bootstrapActionTask: Task<Void, Never>?
    private var gamepadAnnouncementTask: Task<Void, Never>?
    private let apiQueue: DispatchQueue
    private nonisolated let inputWorker: StreamingInputWorker
    private var initialGamepadAnnouncementPending = false
    private var gamepadConnected = false
    private weak var announcedController: GCController?
    private var locallyClosing = false
    private var inputAndHapticsStopped = false
    private var hapticEngines: [IOSStreamHapticsRoute: CHHapticEngine] = [:]
    private var hapticPlayers: [IOSStreamHapticsRoute: CHHapticPatternPlayer] = [:]
    private weak var hapticsController: GCController?

    init(
        inputSession: StreamInputSession = StreamInputSession(),
        frameFeedback: StreamingFrameFeedbackSession? = nil,
        targetVideoWidth: Int = 1_920,
        targetVideoHeight: Int = 1_080,
        apiQueue: DispatchQueue = .main,
        eventSink: @escaping @MainActor @Sendable (String, String) -> Void,
        failureSink: @escaping @MainActor @Sendable (String) -> Void = { _ in }
    ) {
        let relay = StreamingInputEventRelay()
        inputWorker = StreamingInputWorker(feedback: frameFeedback) { relay.send($0) }
        self.apiQueue = apiQueue
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
        super.init()
        relay.bind { [weak self] event in self?.receiveInputWorkerEvent(event) }
    }

    func requestVideoKeyframe() async -> Bool {
        guard !locallyClosing, let channel = channels["control"], channel.readyState == .open else {
            return false
        }
        return await sendText(
            "{\"message\":\"videoKeyframeRequested\",\"ifrRequested\":true}", on: channel)
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
            apiQueue.async { [weak self] in created.channel.delegate = self }
            channels[created.label] = created.channel
            eventSink(created.label, "created")
            if created.label == "input" {
                inputWorker.attach(InputChannelLease(channel: created.channel))
                inputSession.bindWorker(inputWorker)
            }
            if created.channel.readyState == .open,
                let label = StreamingDataChannelLabel(wireLabel: created.label)
            {
                stateMachine.channelDidOpen(label)
            }
        }
    }

    func stopInputAndHaptics() {
        guard !inputAndHapticsStopped else { return }
        inputAndHapticsStopped = true
        frameFeedback?.invalidate()
        delayedGamepadAddedTask?.cancel()
        delayedGamepadAddedTask = nil
        initialGamepadAnnouncementPending = false
        bootstrapActionTask?.cancel()
        bootstrapActionTask = nil
        gamepadAnnouncementTask?.cancel()
        gamepadAnnouncementTask = nil
        inputWorker.stop()
        inputSession.unbindWorker(inputWorker)
        inputStarted = false
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
        let outcome = await inputWorker.stopAndDrain(policy: policy)
        eventSink("input", "neutralFrame\(outcome.rawValue.capitalized)")
        let closingChannels = channels.map {
            XboxStreamCreatedDataChannel(label: $0.key, channel: $0.value)
        }
        await withCheckedContinuation { continuation in
            apiQueue.async {
                for item in closingChannels {
                    item.channel.delegate = nil
                    item.channel.close()
                }
                continuation.resume()
            }
        }
        channels.removeAll()
        eventSink("all", "dataChannelsClosed")
        gamepadConnected = false
        stateMachine = StreamingDataChannelStateMachine(
            postHandshakeCount: postHandshakePayloads.count,
            controlBootstrapCount: controlBootstrapPayloads.count
        )
    }

    nonisolated func debugSnapshot() -> StreamingDataChannelDebugSnapshot { snapshotStore.value }

    private func updateDebugSnapshot() {
        let snapshot = stateMachine.snapshot
        snapshotStore.update(
            StreamingDataChannelDebugSnapshot(
                readyStates: channels.mapValues { Self.readyStateName($0.readyState) },
                phases: Dictionary(
                    uniqueKeysWithValues: snapshot.phases.map {
                        ($0.key.rawValue, $0.value.rawValue)
                    }
                ),
                handshakeAcknowledged: snapshot.handshakeAcknowledged,
                controlReady: snapshot.controlReady,
                inputStarted: snapshot.inputStarted,
                terminalReason: snapshot.terminalReason?.code
            ))
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
                let sent = await self.sendText(
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
        updateDebugSnapshot()
        guard !locallyClosing, !inputAndHapticsStopped, bootstrapActionTask == nil else { return }
        bootstrapRetryTask?.cancel()
        bootstrapRetryTask = nil
        bootstrapActionTask = Task { @MainActor [weak self] in
            guard let self else { return }
            defer {
                self.bootstrapActionTask = nil
                self.updateDebugSnapshot()
            }
            while let action = self.stateMachine.nextAction(), !Task.isCancelled {
                let succeeded: Bool
                switch action {
                case .sendMessageHandshake:
                    if let channel = self.channel(.message) {
                        succeeded = await self.sendText(
                            streamMessageHandshakePayload(), on: channel)
                    } else {
                        succeeded = false
                    }
                    if succeeded { self.eventSink("message", "handshakeSent") }
                case .sendPostHandshake(let index):
                    if let channel = self.channel(.message) {
                        succeeded = await self.sendText(
                            self.postHandshakePayloads[index], on: channel)
                    } else {
                        succeeded = false
                    }
                    if succeeded, index + 1 == self.postHandshakePayloads.count {
                        self.eventSink("message", "postHandshakeCompleted")
                    }
                case .sendControlBootstrap(let stage, let index):
                    if let channel = self.channel(.control) {
                        succeeded = await self.sendText(
                            self.controlBootstrapPayloads[index], on: channel)
                    } else {
                        succeeded = false
                    }
                    if succeeded, index + 1 == self.controlBootstrapPayloads.count {
                        self.eventSink(
                            "control",
                            stage == .preHandshake ? "bootstrapPreHandshakeSent" : "bootstrapSent")
                    }
                case .sendInputMetadata(let stage):
                    succeeded = await self.inputWorker.sendBootstrap(
                        Data(streamInputMetadataBootstrapPayload()))
                    if succeeded {
                        self.eventSink(
                            "input",
                            stage == .preHandshake
                                ? "metadataPreHandshakeSent" : "metadataBootstrapSent")
                    }
                case .announceControlReady:
                    self.eventSink("control", "ready")
                    succeeded = true
                case .scheduleGamepadAnnouncement:
                    if let control = self.channel(.control) {
                        self.scheduleInitialGamepadAnnouncement(on: control)
                        succeeded = true
                    } else {
                        succeeded = false
                    }
                case .startInput:
                    self.startInputLoopIfReady()
                    succeeded = self.inputStarted
                }
                guard !Task.isCancelled, !self.locallyClosing else { return }
                if succeeded {
                    self.stateMachine.actionDidSucceed(action)
                    self.updateDebugSnapshot()
                } else {
                    self.stateMachine.actionDidFail(action)
                    self.eventSink(self.actionChannel(action).rawValue, "bootstrapActionFailed")
                    self.scheduleBootstrapRetry()
                    return
                }
            }
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
        updateDebugSnapshot()
        eventSink(channel, "terminal:\(reason.code)")
        failureSink(reason.code)
    }

    private func sendText(_ payload: String, on channel: RTCDataChannel) async -> Bool {
        let lease = XboxStreamCreatedDataChannel(label: channel.label, channel: channel)
        let sent = await withCheckedContinuation { continuation in
            apiQueue.async {
                continuation.resume(
                    returning: lease.channel.readyState == .open
                        && lease.channel.sendData(
                            RTCDataBuffer(data: Data(payload.utf8), isBinary: false))
                )
            }
        }
        eventSink(channel.label, sent ? "messageSent" : "messageSendFailed")
        return sent
    }

    private func startInputLoopIfReady() {
        let snapshot = stateMachine.snapshot
        guard !inputStarted, snapshot.handshakeAcknowledged, snapshot.controlReady,
            snapshot.postHandshakeInputMetadataSent
        else { return }
        inputStarted = true
        inputWorker.start()
    }

    private func receiveInputWorkerEvent(_ event: StreamingInputWorkerEvent) {
        guard !locallyClosing, !inputAndHapticsStopped else { return }
        inputSession.receiveWorkerEvent(event)
        switch event {
        case .state:
            if inputSession.isSuppressed || !inputSession.preferences.vibrationEnabled {
                resetHaptics()
            }
            let controller = currentGamepadController()
            if hapticsController !== controller { resetHaptics() }
            let connected = controller?.extendedGamepad != nil || inputSession.showsTouchController
            guard !initialGamepadAnnouncementPending, gamepadAnnouncementTask == nil,
                connected != gamepadConnected || controller !== announcedController,
                let control = channels["control"], control.readyState == .open
            else { return }
            gamepadAnnouncementTask = Task { @MainActor [weak self] in
                guard let self else { return }
                defer { self.gamepadAnnouncementTask = nil }
                if await self.sendText(
                    streamControlGamepadChangedPayload(added: connected), on: control),
                    !Task.isCancelled
                {
                    self.gamepadConnected = connected
                    self.announcedController = controller
                    self.eventSink("control", connected ? "gamepadAddedSent" : "gamepadRemovedSent")
                }
            }
        case .rumble(let effects, let epoch):
            guard inputWorker.inputFence.accepts(epoch) else { return }
            handleRumbleEffects(effects)
        case .diagnostic(let reason):
            if inputSession.isSuppressed { resetHaptics() }
            eventSink("input", reason)
        default: break
        }
    }

    private func currentGamepadController() -> GCController? {
        inputSession.currentController
    }

    private func currentGamepad() -> GCExtendedGamepad? {
        currentGamepadController()?.extendedGamepad
    }

    private func handleInputPayload(_ data: Data) { inputWorker.receiveRumble(data) }

    private func handleRumbleEffects(_ effects: [IOSStreamRumbleEffect]) {
        guard !inputSession.isSuppressed, inputSession.preferences.vibrationEnabled else {
            resetHaptics()
            return
        }
        for effect in effects {
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
                defer { self.updateDebugSnapshot() }
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
        if dataChannel.label == "input", buffer.isBinary {
            inputWorker.receiveRumble(data)
            return
        }
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
