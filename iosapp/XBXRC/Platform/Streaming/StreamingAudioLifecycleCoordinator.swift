import AVFoundation
import Foundation

#if canImport(WebRTC)
@preconcurrency import WebRTC

final class StreamingAudioLifecycleCoordinator: NSObject, @unchecked Sendable {
    typealias EventSink = @Sendable (String, [String: IOSRuntimeTraceValue]) -> Void

    private let queue = DispatchQueue(label: "com.xbxrc.ios.streaming.audio-lifecycle")
    private let session = RTCAudioSession.sharedInstance()
    private let eventSink: EventSink
    private var machine = StreamingAudioLifecycleStateMachine()
    private var nextGeneration: UInt64 = 0
    private var activationLeaseCount = 0
    private var observing = false
    private var activeAudioDirection: StreamingMediaDirection = .receiveOnly

    init(eventSink: @escaping EventSink) {
        self.eventSink = eventSink
        super.init()
    }

    func start(audioDirection: StreamingMediaDirection) throws {
        try queue.sync {
            nextGeneration &+= 1
            let generation = nextGeneration
            activeAudioDirection = audioDirection
            if !observing {
                session.add(self)
                observing = true
            }
            try process(.start(generation: generation), audioDirection: audioDirection)
        }
    }

    func stop() {
        queue.sync {
            guard let generation = machine.generation else {
                removeDelegateIfNeeded()
                return
            }
            do {
                try process(.stop(generation: generation))
            } catch {
                emitDirect(
                    "audioSessionDeactivationFailed",
                    payload: errorPayload(error, generation: generation)
                )
                releaseAllActivationLeases()
                removeDelegateIfNeeded()
            }
        }
    }

    private func enqueue(
        _ makeEvent: @escaping @Sendable (UInt64) -> StreamingAudioLifecycleEvent,
        payload: [String: IOSRuntimeTraceValue] = [:]
    ) {
        queue.async { [weak self] in
            guard let self, let generation = self.machine.generation else { return }
            try? self.process(makeEvent(generation), eventPayload: payload)
        }
    }

    private func enqueueDiagnostic(
        _ event: String,
        payload: [String: IOSRuntimeTraceValue] = [:]
    ) {
        queue.async { [weak self] in
            guard let self, let generation = self.machine.generation else { return }
            var contextualPayload = payload
            contextualPayload["audioGeneration"] = .integer(Int64(generation))
            for (key, value) in self.sessionPayload() where contextualPayload[key] == nil {
                contextualPayload[key] = value
            }
            self.eventSink(event, contextualPayload)
        }
    }

    private func process(
        _ event: StreamingAudioLifecycleEvent,
        eventPayload: [String: IOSRuntimeTraceValue] = [:],
        audioDirection: StreamingMediaDirection? = nil
    ) throws {
        let activeAudioDirection = audioDirection ?? self.activeAudioDirection
        var pending = machine.handle(event)
        while !pending.isEmpty {
            let effect = pending.removeFirst()
            switch effect {
            case let .activate(generation, reason):
                do {
                    try activate(reason: reason, audioDirection: activeAudioDirection)
                    pending.append(contentsOf: machine.handle(.activationSucceeded(generation: generation)))
                } catch {
                    pending.append(
                        contentsOf: machine.handle(
                            .activationFailed(
                                generation: generation,
                                failure: Self.errorCategory(error)
                            )
                        )
                    )
                    try drainTraceEffects(&pending, eventPayload: eventPayload)
                    throw error
                }
            case let .deactivate(generation):
                let error = deactivate()
                let result: StreamingAudioLifecycleEvent = if let error {
                    .deactivationFailed(
                        generation: generation,
                        failure: Self.errorCategory(error)
                    )
                } else {
                    .deactivationSucceeded(generation: generation)
                }
                pending.append(contentsOf: machine.handle(result))
            case let .trace(trace):
                emit(trace, eventPayload: eventPayload)
                if trace.event == .audioLifecycleStopped {
                    removeDelegateIfNeeded()
                }
            case .none:
                break
            }
        }
    }

    private func drainTraceEffects(
        _ effects: inout [StreamingAudioLifecycleEffect],
        eventPayload: [String: IOSRuntimeTraceValue]
    ) throws {
        while !effects.isEmpty {
            switch effects.removeFirst() {
            case let .trace(trace):
                emit(trace, eventPayload: eventPayload)
            case .none:
                break
            case .activate, .deactivate:
                throw StreamingAudioLifecycleCoordinatorError.invalidNestedEffect
            }
        }
    }

    private func activate(
        reason: StreamingAudioActivationReason,
        audioDirection: StreamingMediaDirection
    ) throws {
        let configuration = Self.makeConfiguration(for: audioDirection)
        RTCAudioSessionConfiguration.setWebRTC(configuration)

        session.lockForConfiguration()
        defer { session.unlockForConfiguration() }
        var acquiredActivationLease = false
        do {
            session.ignoresPreferredAttributeConfigurationErrors = audioDirection == .receiveOnly
            try session.setConfiguration(configuration)
            if !session.isActive {
                try session.setActive(true)
                activationLeaseCount += 1
                acquiredActivationLease = true
            }
            if audioDirection == .sendReceive {
                // WebRTC 仅在 active + voiceChat 下应用首选输入/输出通道。
                try session.setConfiguration(configuration)
            }
            session.isAudioEnabled = true
        } catch {
            if acquiredActivationLease {
                session.isAudioEnabled = false
                try? session.setActive(false)
                activationLeaseCount = max(0, activationLeaseCount - 1)
            }
            emitDirect(
                "audioSessionConfigurationRejected",
                payload: configurationPayload(configuration, reason: reason, error: error)
            )
            throw error
        }
    }

    static func makeConfiguration(
        for audioDirection: StreamingMediaDirection
    ) -> RTCAudioSessionConfiguration {
        let configuration = RTCAudioSessionConfiguration()
        switch audioDirection {
        case .receiveOnly:
            configuration.category = AVAudioSession.Category.playback.rawValue
            configuration.mode = AVAudioSession.Mode.default.rawValue
            // playback 原生支持 A2DP/AirPlay；显式 allowAirPlay 仅适用于 playAndRecord。
            configuration.categoryOptions = []
        case .sendReceive:
            configuration.category = AVAudioSession.Category.playAndRecord.rawValue
            configuration.mode = AVAudioSession.Mode.voiceChat.rawValue
            configuration.categoryOptions = [
                .allowBluetoothHFP,
                .allowBluetoothA2DP,
                .allowAirPlay,
            ]
        }
        return configuration
    }

    private func deactivate() -> Error? {
        session.lockForConfiguration()
        defer { session.unlockForConfiguration() }
        session.isAudioEnabled = false
        var lastError: Error?
        while activationLeaseCount > 0 {
            do {
                try session.setActive(false)
            } catch {
                lastError = error
            }
            activationLeaseCount -= 1
        }
        return lastError
    }

    private func releaseAllActivationLeases() {
        _ = deactivate()
    }

    private func removeDelegateIfNeeded() {
        guard observing else { return }
        session.remove(self)
        observing = false
    }

    private func emit(
        _ trace: StreamingAudioLifecycleTrace,
        eventPayload: [String: IOSRuntimeTraceValue]
    ) {
        var payload = eventPayload
        payload["audioGeneration"] = .integer(Int64(trace.generation))
        payload["previousState"] = .string(trace.previousState.rawValue)
        payload["state"] = .string(trace.state.rawValue)
        payload["reason"] = trace.reason.map(IOSRuntimeTraceValue.string) ?? .null
        for (key, value) in sessionPayload() where payload[key] == nil {
            payload[key] = value
        }
        eventSink(trace.event.rawValue, payload)
    }

    private func emitDirect(
        _ event: String,
        payload: [String: IOSRuntimeTraceValue]
    ) {
        eventSink(event, payload)
    }

    private func sessionPayload() -> [String: IOSRuntimeTraceValue] {
        let route = session.currentRoute
        return [
            "isActive": .bool(session.isActive),
            "isAudioEnabled": .bool(session.isAudioEnabled),
            "activationLeaseCount": .integer(Int64(activationLeaseCount)),
            "category": .string(session.category),
            "mode": .string(session.mode),
            "outputPortTypes": .array(route.outputs.map { .string($0.portType.rawValue) }),
            "inputPortTypes": .array(route.inputs.map { .string($0.portType.rawValue) }),
            "sampleRate": .double(session.sampleRate),
            "ioBufferDurationMs": .double(session.ioBufferDuration * 1_000),
            "outputLatencyMs": .double(session.outputLatency * 1_000),
            "secondaryAudioShouldBeSilenced": .bool(session.secondaryAudioShouldBeSilencedHint),
        ]
    }

    private func errorPayload(
        _ error: Error,
        generation: UInt64
    ) -> [String: IOSRuntimeTraceValue] {
        let nsError = error as NSError
        return [
            "audioGeneration": .integer(Int64(generation)),
            "errorCategory": .string(Self.errorCategory(error)),
            "errorDomain": .string(nsError.domain),
            "errorCode": .integer(Int64(nsError.code)),
        ]
    }

    private func configurationPayload(
        _ configuration: RTCAudioSessionConfiguration,
        reason: StreamingAudioActivationReason,
        error: Error
    ) -> [String: IOSRuntimeTraceValue] {
        let nsError = error as NSError
        return [
            "errorCategory": .string(Self.errorCategory(error)),
            "errorDomain": .string(nsError.domain),
            "errorCode": .integer(Int64(nsError.code)),
            "activationReason": .string(reason.rawValue),
            "requestedCategory": .string(configuration.category),
            "requestedMode": .string(configuration.mode),
            "requestedCategoryOptions": .integer(Int64(configuration.categoryOptions.rawValue)),
            "requestedInputChannels": .integer(Int64(configuration.inputNumberOfChannels)),
            "requestedOutputChannels": .integer(Int64(configuration.outputNumberOfChannels)),
            "requestedSampleRate": .double(configuration.sampleRate),
            "requestedIOBufferDurationMs": .double(configuration.ioBufferDuration * 1_000),
        ]
    }

    private static func errorCategory(_ error: Error) -> String {
        let nsError = error as NSError
        // 当前预编译 WebRTC 头文件声明了错误域常量，二进制未导出对应符号。
        if nsError.domain.hasPrefix("org.webrtc.")
            && nsError.domain.contains("RTCAudioSession")
        {
            return "rtcAudioSession"
        }
        if nsError.domain == AVFoundationErrorDomain {
            return "avFoundation"
        }
        return "system"
    }

    private static func routeChangeReason(
        _ reason: AVAudioSession.RouteChangeReason
    ) -> StreamingAudioRouteChangeReason {
        switch reason {
        case .newDeviceAvailable: .newDeviceAvailable
        case .oldDeviceUnavailable: .oldDeviceUnavailable
        case .categoryChange: .categoryChange
        case .override: .override
        case .wakeFromSleep: .wakeFromSleep
        case .noSuitableRouteForCategory: .noSuitableRoute
        case .routeConfigurationChange: .routeConfigurationChange
        case .unknown: .unknown
        @unknown default: .unknown
        }
    }
}

extension StreamingAudioLifecycleCoordinator: RTCAudioSessionDelegate {
    func audioSessionDidBeginInterruption(_: RTCAudioSession) {
        enqueue { .interruptionBegan(generation: $0) }
    }

    func audioSessionDidEndInterruption(
        _: RTCAudioSession,
        shouldResumeSession: Bool
    ) {
        enqueue(
            { .interruptionEnded(generation: $0, shouldResume: shouldResumeSession) },
            payload: ["shouldResume": .bool(shouldResumeSession)]
        )
    }

    func audioSessionDidChangeRoute(
        _ session: RTCAudioSession,
        reason: AVAudioSession.RouteChangeReason,
        previousRoute: AVAudioSessionRouteDescription
    ) {
        let projectedReason = Self.routeChangeReason(reason)
        let hasOutput = !session.currentRoute.outputs.isEmpty
        enqueue(
            {
                .routeChanged(
                    generation: $0,
                    reason: projectedReason,
                    hasOutput: hasOutput
                )
            },
            payload: [
                "routeChangeReason": .string(projectedReason.rawValue),
                "hasOutput": .bool(hasOutput),
                "previousOutputPortTypes": .array(
                    previousRoute.outputs.map { .string($0.portType.rawValue) }
                ),
            ]
        )
    }

    func audioSessionMediaServerTerminated(_: RTCAudioSession) {
        enqueue { .mediaServicesLost(generation: $0) }
    }

    func audioSessionMediaServerReset(_: RTCAudioSession) {
        enqueue { .mediaServicesReset(generation: $0) }
    }

    func audioSessionDidStartPlayOrRecord(_: RTCAudioSession) {
        enqueueDiagnostic("audioUnitStarted")
    }

    func audioSessionDidStopPlayOrRecord(_: RTCAudioSession) {
        enqueueDiagnostic("audioUnitStopped")
    }

    func audioSession(
        _: RTCAudioSession,
        didDetectPlayoutGlitch totalNumberOfGlitches: Int64
    ) {
        enqueueDiagnostic(
            "audioPlayoutGlitchDetected",
            payload: ["totalGlitchCount": .integer(totalNumberOfGlitches)]
        )
    }

    func audioSession(
        _: RTCAudioSession,
        failedToSetActive active: Bool,
        error: Error
    ) {
        let nsError = error as NSError
        enqueueDiagnostic(
            "audioSessionNativeSetActiveFailed",
            payload: [
                "requestedActive": .bool(active),
                "errorCategory": .string(Self.errorCategory(error)),
                "errorDomain": .string(nsError.domain),
                "errorCode": .integer(Int64(nsError.code)),
            ]
        )
    }

    func audioSession(
        _: RTCAudioSession,
        audioUnitStartFailedWithError error: Error
    ) {
        let nsError = error as NSError
        enqueueDiagnostic(
            "audioUnitStartFailed",
            payload: [
                "errorCategory": .string(Self.errorCategory(error)),
                "errorDomain": .string(nsError.domain),
                "errorCode": .integer(Int64(nsError.code)),
            ]
        )
    }
}

private enum StreamingAudioLifecycleCoordinatorError: Error {
    case invalidNestedEffect
}
#endif
