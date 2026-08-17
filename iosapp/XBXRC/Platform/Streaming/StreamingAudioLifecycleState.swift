enum StreamingAudioLifecycleState: String, Equatable, Sendable {
    case idle
    case activating
    case active
    case interrupted
    case mediaServicesLost
    case recovering
    case failed
    case stopping
}

enum StreamingAudioActivationReason: String, Equatable, Sendable {
    case initialStart
    case interruptionEnded
    case routeUnavailable
    case mediaServicesReset
    case retryAfterFailure
}

enum StreamingAudioRouteChangeReason: String, Equatable, Sendable {
    case newDeviceAvailable
    case oldDeviceUnavailable
    case categoryChange
    case override
    case wakeFromSleep
    case noSuitableRoute
    case routeConfigurationChange
    case unknown
}

enum StreamingAudioLifecycleEvent: Equatable, Sendable {
    case start(generation: UInt64)
    case activationSucceeded(generation: UInt64)
    case activationFailed(generation: UInt64, failure: String)
    case interruptionBegan(generation: UInt64)
    case interruptionEnded(generation: UInt64, shouldResume: Bool)
    case routeChanged(
        generation: UInt64,
        reason: StreamingAudioRouteChangeReason,
        hasOutput: Bool
    )
    case mediaServicesLost(generation: UInt64)
    case mediaServicesReset(generation: UInt64)
    case stop(generation: UInt64)
    case deactivationSucceeded(generation: UInt64)
    case deactivationFailed(generation: UInt64, failure: String)

    var generation: UInt64 {
        switch self {
        case let .start(generation),
             let .activationSucceeded(generation),
             let .activationFailed(generation, _),
             let .interruptionBegan(generation),
             let .interruptionEnded(generation, _),
             let .routeChanged(generation, _, _),
             let .mediaServicesLost(generation),
             let .mediaServicesReset(generation),
             let .stop(generation),
             let .deactivationSucceeded(generation),
             let .deactivationFailed(generation, _):
            generation
        }
    }
}

enum StreamingAudioLifecycleTraceEvent: String, Equatable, Sendable {
    case audioLifecycleStarted
    case audioSessionActivationStarted
    case audioSessionActivationSucceeded
    case audioSessionActivationFailed
    case audioInterruptionBegan
    case audioInterruptionEnded
    case audioRouteChanged
    case audioMediaServicesLost
    case audioMediaServicesReset
    case audioRecoveryStarted
    case audioRecoverySucceeded
    case audioRecoveryFailed
    case audioRecoverySkipped
    case audioSessionDeactivationStarted
    case audioSessionDeactivationSucceeded
    case audioSessionDeactivationFailed
    case audioLifecycleStopped
}

struct StreamingAudioLifecycleTrace: Equatable, Sendable {
    let event: StreamingAudioLifecycleTraceEvent
    let generation: UInt64
    let previousState: StreamingAudioLifecycleState
    let state: StreamingAudioLifecycleState
    let reason: String?
}

enum StreamingAudioLifecycleEffect: Equatable, Sendable {
    case activate(generation: UInt64, reason: StreamingAudioActivationReason)
    case deactivate(generation: UInt64)
    case trace(StreamingAudioLifecycleTrace)
    case none
}

struct StreamingAudioLifecycleStateMachine: Equatable, Sendable {
    private(set) var state: StreamingAudioLifecycleState = .idle
    private(set) var generation: UInt64?

    private var lastRouteChangeReason: StreamingAudioRouteChangeReason?
    private var lastRouteOutputAvailable: Bool?

    mutating func handle(_ event: StreamingAudioLifecycleEvent) -> [StreamingAudioLifecycleEffect] {
        if case let .start(eventGeneration) = event {
            return handleStart(generation: eventGeneration)
        }

        guard event.generation == generation else { return [.none] }

        switch event {
        case .start:
            return [.none]
        case .activationSucceeded:
            return handleActivationSucceeded()
        case let .activationFailed(_, failure):
            return handleActivationFailed(failure: failure)
        case .interruptionBegan:
            return handleInterruptionBegan()
        case let .interruptionEnded(_, shouldResume):
            return handleInterruptionEnded(shouldResume: shouldResume)
        case let .routeChanged(_, reason, hasOutput):
            return handleRouteChanged(reason: reason, hasOutput: hasOutput)
        case .mediaServicesLost:
            return handleMediaServicesLost()
        case .mediaServicesReset:
            return handleMediaServicesReset()
        case .stop:
            return handleStop()
        case .deactivationSucceeded:
            return handleDeactivationSucceeded()
        case let .deactivationFailed(_, failure):
            return handleDeactivationFailed(failure: failure)
        }
    }

    private mutating func handleStart(generation eventGeneration: UInt64) -> [StreamingAudioLifecycleEffect] {
        if let generation, eventGeneration <= generation { return [.none] }

        let previousState = state
        generation = eventGeneration
        state = .activating
        lastRouteChangeReason = nil
        lastRouteOutputAvailable = nil

        return [
            trace(.audioLifecycleStarted, from: previousState),
            trace(.audioSessionActivationStarted, from: previousState, reason: StreamingAudioActivationReason.initialStart.rawValue),
            .activate(generation: eventGeneration, reason: .initialStart),
        ]
    }

    private mutating func handleActivationSucceeded() -> [StreamingAudioLifecycleEffect] {
        let previousState = state
        switch state {
        case .activating:
            state = .active
            return [trace(.audioSessionActivationSucceeded, from: previousState)]
        case .recovering:
            state = .active
            return [
                trace(.audioSessionActivationSucceeded, from: previousState),
                trace(.audioRecoverySucceeded, from: previousState),
            ]
        case .idle, .active, .interrupted, .mediaServicesLost, .failed, .stopping:
            return [.none]
        }
    }

    private mutating func handleActivationFailed(failure: String) -> [StreamingAudioLifecycleEffect] {
        let previousState = state
        switch state {
        case .activating:
            state = .failed
            return [trace(.audioSessionActivationFailed, from: previousState, reason: failure)]
        case .recovering:
            state = .failed
            return [
                trace(.audioSessionActivationFailed, from: previousState, reason: failure),
                trace(.audioRecoveryFailed, from: previousState, reason: failure),
            ]
        case .idle, .active, .interrupted, .mediaServicesLost, .failed, .stopping:
            return [.none]
        }
    }

    private mutating func handleInterruptionBegan() -> [StreamingAudioLifecycleEffect] {
        let previousState = state
        switch state {
        case .activating, .active, .recovering, .failed:
            state = .interrupted
            return [trace(.audioInterruptionBegan, from: previousState)]
        case .idle, .interrupted, .mediaServicesLost, .stopping:
            return [.none]
        }
    }

    private mutating func handleInterruptionEnded(shouldResume: Bool) -> [StreamingAudioLifecycleEffect] {
        guard state == .interrupted else { return [.none] }

        let previousState = state
        if shouldResume {
            state = .recovering
            return [
                trace(.audioInterruptionEnded, from: previousState, reason: "shouldResume"),
                trace(.audioRecoveryStarted, from: previousState, reason: StreamingAudioActivationReason.interruptionEnded.rawValue),
                trace(.audioSessionActivationStarted, from: previousState, reason: StreamingAudioActivationReason.interruptionEnded.rawValue),
                activate(reason: .interruptionEnded),
            ]
        }

        state = .failed
        return [
            trace(.audioInterruptionEnded, from: previousState, reason: "resumeDenied"),
            trace(.audioRecoverySkipped, from: previousState, reason: "resumeDenied"),
        ]
    }

    private mutating func handleRouteChanged(
        reason: StreamingAudioRouteChangeReason,
        hasOutput: Bool
    ) -> [StreamingAudioLifecycleEffect] {
        if lastRouteChangeReason == reason, lastRouteOutputAvailable == hasOutput { return [.none] }
        lastRouteChangeReason = reason
        lastRouteOutputAvailable = hasOutput

        let previousState = state
        let routeTrace = trace(
            .audioRouteChanged,
            from: previousState,
            reason: "\(reason.rawValue):\(hasOutput ? "outputAvailable" : "outputUnavailable")"
        )

        if state == .active, !hasOutput {
            state = .recovering
            return [
                routeTrace,
                trace(.audioRecoveryStarted, from: previousState, reason: StreamingAudioActivationReason.routeUnavailable.rawValue),
                trace(.audioSessionActivationStarted, from: previousState, reason: StreamingAudioActivationReason.routeUnavailable.rawValue),
                activate(reason: .routeUnavailable),
            ]
        }

        if state == .failed, hasOutput {
            state = .recovering
            return [
                routeTrace,
                trace(.audioRecoveryStarted, from: previousState, reason: StreamingAudioActivationReason.retryAfterFailure.rawValue),
                trace(.audioSessionActivationStarted, from: previousState, reason: StreamingAudioActivationReason.retryAfterFailure.rawValue),
                activate(reason: .retryAfterFailure),
            ]
        }

        switch state {
        case .activating, .active, .interrupted, .recovering, .failed:
            return [routeTrace]
        case .idle, .mediaServicesLost, .stopping:
            return [.none]
        }
    }

    private mutating func handleMediaServicesLost() -> [StreamingAudioLifecycleEffect] {
        let previousState = state
        switch state {
        case .activating, .active, .interrupted, .recovering, .failed:
            state = .mediaServicesLost
            lastRouteChangeReason = nil
            lastRouteOutputAvailable = nil
            return [trace(.audioMediaServicesLost, from: previousState)]
        case .idle, .mediaServicesLost, .stopping:
            return [.none]
        }
    }

    private mutating func handleMediaServicesReset() -> [StreamingAudioLifecycleEffect] {
        guard state == .mediaServicesLost else { return [.none] }

        let previousState = state
        state = .recovering
        return [
            trace(.audioMediaServicesReset, from: previousState),
            trace(.audioRecoveryStarted, from: previousState, reason: StreamingAudioActivationReason.mediaServicesReset.rawValue),
            trace(.audioSessionActivationStarted, from: previousState, reason: StreamingAudioActivationReason.mediaServicesReset.rawValue),
            activate(reason: .mediaServicesReset),
        ]
    }

    private mutating func handleStop() -> [StreamingAudioLifecycleEffect] {
        let previousState = state
        switch state {
        case .activating, .active, .interrupted, .mediaServicesLost, .recovering, .failed:
            state = .stopping
            return [
                trace(.audioSessionDeactivationStarted, from: previousState),
                deactivate(),
            ]
        case .idle, .stopping:
            return [.none]
        }
    }

    private mutating func handleDeactivationSucceeded() -> [StreamingAudioLifecycleEffect] {
        guard state == .stopping else { return [.none] }

        let previousState = state
        state = .idle
        lastRouteChangeReason = nil
        lastRouteOutputAvailable = nil
        return [
            trace(.audioSessionDeactivationSucceeded, from: previousState),
            trace(.audioLifecycleStopped, from: previousState),
        ]
    }

    private mutating func handleDeactivationFailed(failure: String) -> [StreamingAudioLifecycleEffect] {
        guard state == .stopping else { return [.none] }

        let previousState = state
        state = .idle
        lastRouteChangeReason = nil
        lastRouteOutputAvailable = nil
        return [
            trace(.audioSessionDeactivationFailed, from: previousState, reason: failure),
            trace(.audioLifecycleStopped, from: previousState, reason: failure),
        ]
    }

    private func activate(reason: StreamingAudioActivationReason) -> StreamingAudioLifecycleEffect {
        guard let generation else { return .none }
        return .activate(generation: generation, reason: reason)
    }

    private func deactivate() -> StreamingAudioLifecycleEffect {
        guard let generation else { return .none }
        return .deactivate(generation: generation)
    }

    private func trace(
        _ event: StreamingAudioLifecycleTraceEvent,
        from previousState: StreamingAudioLifecycleState,
        reason: String? = nil
    ) -> StreamingAudioLifecycleEffect {
        guard let generation else { return .none }
        return .trace(
            StreamingAudioLifecycleTrace(
                event: event,
                generation: generation,
                previousState: previousState,
                state: state,
                reason: reason
            )
        )
    }
}
