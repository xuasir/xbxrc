import XCTest
@testable import XBXRC

final class StreamingAudioLifecycleStateTests: XCTestCase {
    func testNormalActivationAndStop() {
        var machine = StreamingAudioLifecycleStateMachine()

        XCTAssertEqual(actions(machine.handle(.start(generation: 1))), [
            .activate(generation: 1, reason: .initialStart),
        ])
        XCTAssertEqual(machine.state, .activating)

        XCTAssertTrue(actions(machine.handle(.activationSucceeded(generation: 1))).isEmpty)
        XCTAssertEqual(machine.state, .active)

        XCTAssertEqual(actions(machine.handle(.stop(generation: 1))), [.deactivate(generation: 1)])
        XCTAssertEqual(machine.state, .stopping)

        XCTAssertTrue(actions(machine.handle(.deactivationSucceeded(generation: 1))).isEmpty)
        XCTAssertEqual(machine.state, .idle)
        XCTAssertEqual(traceEvents(machine.handle(.stop(generation: 1))), [])
    }

    func testInterruptionResumesOnceWhenSystemAllowsIt() {
        var machine = activeMachine(generation: 8)

        XCTAssertEqual(traceEvents(machine.handle(.interruptionBegan(generation: 8))), [.audioInterruptionBegan])
        XCTAssertEqual(machine.state, .interrupted)
        XCTAssertEqual(machine.handle(.interruptionBegan(generation: 8)), [.none])

        let recovery = machine.handle(.interruptionEnded(generation: 8, shouldResume: true))
        XCTAssertEqual(actions(recovery), [.activate(generation: 8, reason: .interruptionEnded)])
        XCTAssertEqual(
            traceEvents(recovery),
            [.audioInterruptionEnded, .audioRecoveryStarted, .audioSessionActivationStarted]
        )
        XCTAssertEqual(machine.state, .recovering)
        XCTAssertEqual(machine.handle(.interruptionEnded(generation: 8, shouldResume: true)), [.none])

        XCTAssertEqual(
            traceEvents(machine.handle(.activationSucceeded(generation: 8))),
            [.audioSessionActivationSucceeded, .audioRecoverySucceeded]
        )
        XCTAssertEqual(machine.state, .active)
    }

    func testInterruptionWithoutResumePermissionFailsClosed() {
        var machine = activeMachine(generation: 9)
        _ = machine.handle(.interruptionBegan(generation: 9))

        let effects = machine.handle(.interruptionEnded(generation: 9, shouldResume: false))

        XCTAssertTrue(actions(effects).isEmpty)
        XCTAssertEqual(traceEvents(effects), [.audioInterruptionEnded, .audioRecoverySkipped])
        XCTAssertEqual(machine.state, .failed)
    }

    func testMediaServicesResetRecoversOnce() {
        var machine = activeMachine(generation: 3)

        XCTAssertEqual(traceEvents(machine.handle(.mediaServicesLost(generation: 3))), [.audioMediaServicesLost])
        XCTAssertEqual(machine.state, .mediaServicesLost)
        XCTAssertEqual(machine.handle(.mediaServicesLost(generation: 3)), [.none])

        let effects = machine.handle(.mediaServicesReset(generation: 3))
        XCTAssertEqual(actions(effects), [.activate(generation: 3, reason: .mediaServicesReset)])
        XCTAssertEqual(
            traceEvents(effects),
            [.audioMediaServicesReset, .audioRecoveryStarted, .audioSessionActivationStarted]
        )
        XCTAssertEqual(machine.state, .recovering)
        XCTAssertEqual(machine.handle(.mediaServicesReset(generation: 3)), [.none])
    }

    func testRouteLossStartsOneControlledRecovery() {
        var machine = activeMachine(generation: 5)

        let effects = machine.handle(
            .routeChanged(generation: 5, reason: .oldDeviceUnavailable, hasOutput: false)
        )

        XCTAssertEqual(actions(effects), [.activate(generation: 5, reason: .routeUnavailable)])
        XCTAssertEqual(
            traceEvents(effects),
            [.audioRouteChanged, .audioRecoveryStarted, .audioSessionActivationStarted]
        )
        XCTAssertEqual(machine.state, .recovering)
        XCTAssertEqual(
            machine.handle(.routeChanged(generation: 5, reason: .oldDeviceUnavailable, hasOutput: false)),
            [.none]
        )
    }

    func testRouteReasonChangeRemainsObservableWhenOutputStaysAvailable() {
        var machine = activeMachine(generation: 6)

        XCTAssertEqual(
            traceEvents(
                machine.handle(.routeChanged(generation: 6, reason: .newDeviceAvailable, hasOutput: true))
            ),
            [.audioRouteChanged]
        )
        XCTAssertEqual(
            traceEvents(
                machine.handle(.routeChanged(generation: 6, reason: .categoryChange, hasOutput: true))
            ),
            [.audioRouteChanged]
        )
        XCTAssertEqual(
            machine.handle(.routeChanged(generation: 6, reason: .categoryChange, hasOutput: true)),
            [.none]
        )
        XCTAssertEqual(machine.state, .active)
    }

    func testStaleGenerationCannotMutateOrRecoverCurrentSession() {
        var machine = activeMachine(generation: 11)

        XCTAssertEqual(machine.handle(.interruptionBegan(generation: 10)), [.none])
        XCTAssertEqual(machine.handle(.mediaServicesLost(generation: 10)), [.none])
        XCTAssertEqual(machine.handle(.stop(generation: 10)), [.none])
        XCTAssertEqual(machine.state, .active)
        XCTAssertEqual(machine.generation, 11)
    }

    func testLateEventsAfterStopCannotRecoverAudio() {
        var machine = activeMachine(generation: 21)
        _ = machine.handle(.stop(generation: 21))

        XCTAssertEqual(machine.handle(.interruptionEnded(generation: 21, shouldResume: true)), [.none])
        XCTAssertEqual(machine.handle(.mediaServicesReset(generation: 21)), [.none])
        XCTAssertEqual(
            machine.handle(.routeChanged(generation: 21, reason: .oldDeviceUnavailable, hasOutput: false)),
            [.none]
        )
        XCTAssertEqual(machine.handle(.activationSucceeded(generation: 21)), [.none])
        XCTAssertEqual(machine.state, .stopping)

        _ = machine.handle(.deactivationSucceeded(generation: 21))
        XCTAssertEqual(machine.handle(.mediaServicesReset(generation: 21)), [.none])
        XCTAssertEqual(machine.state, .idle)
    }

    func testDuplicateStartAndCompletionEventsAreIdempotent() {
        var machine = StreamingAudioLifecycleStateMachine()

        _ = machine.handle(.start(generation: 34))
        XCTAssertEqual(machine.handle(.start(generation: 34)), [.none])
        _ = machine.handle(.activationSucceeded(generation: 34))
        XCTAssertEqual(machine.handle(.activationSucceeded(generation: 34)), [.none])
        _ = machine.handle(.stop(generation: 34))
        XCTAssertEqual(machine.handle(.stop(generation: 34)), [.none])
        _ = machine.handle(.deactivationSucceeded(generation: 34))
        XCTAssertEqual(machine.handle(.deactivationSucceeded(generation: 34)), [.none])
    }

    func testRecoveryFailureCanRetryWhenOutputReturns() {
        var machine = activeMachine(generation: 55)
        _ = machine.handle(.routeChanged(generation: 55, reason: .oldDeviceUnavailable, hasOutput: false))

        let failure = machine.handle(.activationFailed(generation: 55, failure: "configuration"))
        XCTAssertEqual(
            traceEvents(failure),
            [.audioSessionActivationFailed, .audioRecoveryFailed]
        )
        XCTAssertEqual(machine.state, .failed)

        let retry = machine.handle(.routeChanged(generation: 55, reason: .newDeviceAvailable, hasOutput: true))
        XCTAssertEqual(actions(retry), [.activate(generation: 55, reason: .retryAfterFailure)])
        XCTAssertEqual(machine.state, .recovering)
    }

    func testNewGenerationCanStartWhilePreviousGenerationIsStopping() {
        var machine = activeMachine(generation: 89)
        _ = machine.handle(.stop(generation: 89))

        XCTAssertEqual(
            actions(machine.handle(.start(generation: 90))),
            [.activate(generation: 90, reason: .initialStart)]
        )
        XCTAssertEqual(machine.state, .activating)
        XCTAssertEqual(machine.generation, 90)
        XCTAssertEqual(machine.handle(.deactivationSucceeded(generation: 89)), [.none])
        XCTAssertEqual(machine.state, .activating)
    }

    func testDeactivationFailureStillClosesLifecycle() {
        var machine = activeMachine(generation: 144)
        _ = machine.handle(.stop(generation: 144))

        XCTAssertEqual(
            traceEvents(machine.handle(.deactivationFailed(generation: 144, failure: "system"))),
            [.audioSessionDeactivationFailed, .audioLifecycleStopped]
        )
        XCTAssertEqual(machine.state, .idle)
    }

    private func activeMachine(generation: UInt64) -> StreamingAudioLifecycleStateMachine {
        var machine = StreamingAudioLifecycleStateMachine()
        _ = machine.handle(.start(generation: generation))
        _ = machine.handle(.activationSucceeded(generation: generation))
        return machine
    }

    private func actions(_ effects: [StreamingAudioLifecycleEffect]) -> [StreamingAudioLifecycleEffect] {
        effects.filter {
            switch $0 {
            case .activate, .deactivate:
                true
            case .trace, .none:
                false
            }
        }
    }

    private func traceEvents(
        _ effects: [StreamingAudioLifecycleEffect]
    ) -> [StreamingAudioLifecycleTraceEvent] {
        effects.compactMap {
            guard case let .trace(trace) = $0 else { return nil }
            return trace.event
        }
    }
}
