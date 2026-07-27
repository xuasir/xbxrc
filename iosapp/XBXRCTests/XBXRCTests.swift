import XCTest
@testable import XBXRC

final class XBXRCTests: XCTestCase {
    func testStreamingMediaSampleTrackerOnlyAdvancesOnPositiveDeltas() {
        var tracker = StreamingMediaSampleTracker()

        let first = tracker.observe(
            inboundVideoBytes: 1_000,
            packetsReceived: 10,
            framesDecoded: 1,
            observedAtMilliseconds: 1_000
        )
        XCTAssertTrue(first.mediaAdvanced)
        XCTAssertEqual(first.firstMediaAtMilliseconds, 1_000)
        XCTAssertEqual(first.lastMediaAtMilliseconds, 1_000)
        XCTAssertNil(first.frameSupplyDelta)

        let stalled = tracker.observe(
            inboundVideoBytes: 1_000,
            packetsReceived: 10,
            framesDecoded: 1,
            observedAtMilliseconds: 2_000
        )
        XCTAssertFalse(stalled.mediaAdvanced)
        XCTAssertEqual(stalled.firstMediaAtMilliseconds, 1_000)
        XCTAssertEqual(stalled.lastMediaAtMilliseconds, 1_000)
        XCTAssertEqual(stalled.frameSupplyDelta, 0)

        let advanced = tracker.observe(
            inboundVideoBytes: 1_500,
            packetsReceived: 14,
            framesDecoded: 2,
            observedAtMilliseconds: 3_000
        )
        XCTAssertTrue(advanced.mediaAdvanced)
        XCTAssertEqual(advanced.lastMediaAtMilliseconds, 3_000)
        XCTAssertEqual(advanced.frameSupplyDelta, 1)

        let countersReset = tracker.observe(
            inboundVideoBytes: 20,
            packetsReceived: 1,
            framesDecoded: 0,
            observedAtMilliseconds: 4_000
        )
        XCTAssertFalse(countersReset.mediaAdvanced)
        XCTAssertEqual(countersReset.lastMediaAtMilliseconds, 3_000)
        XCTAssertNil(countersReset.frameSupplyDelta)
    }

    func testIOSStreamInputPacketMatchesXboxGamepadWireLayout() {
        let packet = IOSStreamInputPacketEncoder.encodeGamepad(
            sequence: 0x01020304,
            timestampMilliseconds: 1_000,
            state: IOSStreamGamepadState(
                buttonMask: 0xA55A,
                leftThumbX: 0.5,
                leftThumbY: -0.5,
                rightThumbX: -1,
                rightThumbY: 1,
                leftTrigger: 0.25,
                rightTrigger: 1
            )
        )
        let bytes = [UInt8](packet)

        XCTAssertEqual(bytes, [
            2, 0,
            4, 3, 2, 1,
            0, 0, 0, 0, 0, 64, 143, 64,
            1,
            0,
            90, 165,
            0, 64,
            0, 192,
            1, 128,
            255, 127,
            0, 64,
            255, 255,
            0, 0, 0, 0,
            0, 0, 0, 0,
        ])
        XCTAssertEqual(Array(bytes[0 ..< 6]), [2, 0, 4, 3, 2, 1])
        XCTAssertEqual(bytes[14], 1)
        XCTAssertEqual(bytes[15], 0)
        XCTAssertEqual(readUInt16LE(bytes, at: 16), 0xA55A)
        XCTAssertEqual(readInt16LE(bytes, at: 18), 16_384)
        XCTAssertEqual(readInt16LE(bytes, at: 20), -16_384)
        XCTAssertEqual(readInt16LE(bytes, at: 22), -32_767)
        XCTAssertEqual(readInt16LE(bytes, at: 24), 32_767)
        XCTAssertEqual(readUInt16LE(bytes, at: 26), 16_384)
        XCTAssertEqual(readUInt16LE(bytes, at: 28), 65_535)
        XCTAssertEqual(Array(bytes[30 ..< 38]), Array(repeating: UInt8(0), count: 8))
    }

    func testIOSStreamInputPacketClampsAxesTriggersAndEncodesNeutral() {
        let saturated = IOSStreamInputPacketEncoder.encodeGamepad(
            sequence: 1,
            timestampMilliseconds: -10,
            state: IOSStreamGamepadState(
                leftThumbX: 2,
                leftThumbY: -2,
                rightThumbX: -2,
                rightThumbY: 2,
                leftTrigger: -1,
                rightTrigger: 2
            )
        )
        let bytes = [UInt8](saturated)
        XCTAssertEqual(readInt16LE(bytes, at: 18), 32_767)
        XCTAssertEqual(readInt16LE(bytes, at: 20), -32_767)
        XCTAssertEqual(readInt16LE(bytes, at: 22), -32_767)
        XCTAssertEqual(readInt16LE(bytes, at: 24), 32_767)
        XCTAssertEqual(readUInt16LE(bytes, at: 26), 0)
        XCTAssertEqual(readUInt16LE(bytes, at: 28), 65_535)

        let neutral = IOSStreamInputPacketEncoder.encodeGamepad(
            sequence: 2,
            timestampMilliseconds: 0,
            state: .neutral
        )
        XCTAssertEqual(Array(neutral.suffix(23)), Array(repeating: UInt8(0), count: 23))
    }

    func testIOSStreamInputSendGateUsesChangeKeepaliveAndBackpressureHysteresis() {
        var gate = IOSStreamInputSendGate()
        let pressed = IOSStreamGamepadState(buttonMask: 16)

        XCTAssertTrue(gate.shouldSend(state: pressed, bufferedAmount: 0, nowMilliseconds: 10))
        gate.markSent(state: pressed, at: 10)
        XCTAssertFalse(gate.shouldSend(state: pressed, bufferedAmount: 0, nowMilliseconds: 200))
        XCTAssertTrue(gate.shouldSend(state: pressed, bufferedAmount: 0, nowMilliseconds: 260))

        XCTAssertFalse(gate.shouldSend(
            state: .neutral,
            bufferedAmount: IOSStreamInputSendGate.highWatermarkBytes,
            nowMilliseconds: 300
        ))
        XCTAssertTrue(gate.blocked)
        XCTAssertFalse(gate.shouldSend(
            state: .neutral,
            bufferedAmount: IOSStreamInputSendGate.lowWatermarkBytes + 1,
            nowMilliseconds: 310
        ))
        XCTAssertTrue(gate.shouldSend(
            state: .neutral,
            bufferedAmount: IOSStreamInputSendGate.lowWatermarkBytes,
            nowMilliseconds: 320
        ))
        XCTAssertFalse(gate.blocked)
    }

    func testIOSStreamInputDrainPolicyWaitsForLowWatermarkAndHasFixedUpperBound() {
        let policy = IOSStreamInputDrainPolicy.standard

        XCTAssertFalse(policy.shouldEnqueueNeutral(
            bufferedAmount: policy.lowWatermarkBytes + 1,
            elapsedMilliseconds: policy.maximumWaitMilliseconds - 1
        ))
        XCTAssertTrue(policy.shouldEnqueueNeutral(
            bufferedAmount: policy.lowWatermarkBytes,
            elapsedMilliseconds: 0
        ))
        XCTAssertTrue(policy.shouldEnqueueNeutral(
            bufferedAmount: UInt64.max,
            elapsedMilliseconds: policy.maximumWaitMilliseconds
        ))
        XCTAssertEqual(
            policy.maximumTotalMilliseconds,
            policy.maximumWaitMilliseconds + policy.postNeutralGraceMilliseconds
        )
        XCTAssertLessThanOrEqual(policy.maximumTotalMilliseconds, 100)
        XCTAssertEqual(IOSStreamNeutralFrameOutcome.sendFailed.rawValue, "sendFailed")
    }

    func testIOSStreamRumbleDecoderPreservesBetterXcloudFourMotorSemantics() {
        let effects = IOSStreamRumblePacketDecoder.decode(
            Data([128, 0, 0, 2, 80, 40, 30, 20, 120, 0, 0, 0, 0])
        )

        XCTAssertEqual(effects.count, 1)
        XCTAssertEqual(effects[0].format, .betterXcloud)
        XCTAssertEqual(effects[0].gamepadIndex, 2)
        XCTAssertEqual(effects[0].strongMagnitude, 0.8, accuracy: 0.001)
        XCTAssertEqual(effects[0].weakMagnitude, 0.4, accuracy: 0.001)
        XCTAssertEqual(effects[0].leftTrigger, 0.3, accuracy: 0.001)
        XCTAssertEqual(effects[0].rightTrigger, 0.2, accuracy: 0.001)
        XCTAssertEqual(effects[0].durationMilliseconds, 120)
    }

    func testIOSStreamRumbleDecoderReadsLegacyRecordsAndStopPacket() {
        let record: [UInt8] = [1, 1, 0xFF, 0x03, 0, 0, 0, 2, 0xFF, 0x03, 50, 0]
        let effects = IOSStreamRumblePacketDecoder.decode(Data([128, 0] + record + record))

        XCTAssertEqual(effects.count, 2)
        XCTAssertEqual(effects[0].format, .legacy)
        XCTAssertEqual(effects[0].gamepadIndex, 1)
        XCTAssertEqual(effects[0].leftTrigger, 1, accuracy: 0.001)
        XCTAssertEqual(effects[0].weakMagnitude, Float(512) / 1_023, accuracy: 0.001)
        XCTAssertEqual(effects[0].strongMagnitude, 1, accuracy: 0.001)
        XCTAssertEqual(effects[0].durationMilliseconds, 50)

        let stop = IOSStreamRumblePacketDecoder.decode(
            Data([128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
        )
        XCTAssertEqual(stop.count, 1)
        XCTAssertTrue(stop[0].isStop)
    }

    func testIOSStreamHapticsRouterKeepsFourLocalitiesAndDegradesDeterministically() {
        let effect = IOSStreamRumbleEffect(
            gamepadIndex: 0,
            strongMagnitude: 0.9,
            weakMagnitude: 0.4,
            leftTrigger: 0.3,
            rightTrigger: 0.2,
            durationMilliseconds: 80,
            format: .betterXcloud
        )
        let fourRoutes = IOSStreamHapticsRouter.pulses(
            for: effect,
            supportedRoutes: [.leftHandle, .rightHandle, .leftTrigger, .rightTrigger]
        )
        XCTAssertEqual(fourRoutes.map(\.route), [
            .leftHandle, .rightHandle, .leftTrigger, .rightTrigger,
        ])

        let grouped = IOSStreamHapticsRouter.pulses(
            for: effect,
            supportedRoutes: [.handles, .triggers]
        )
        XCTAssertEqual(grouped.map(\.route), [.handles, .triggers])
        XCTAssertEqual(grouped[0].intensity, 0.9, accuracy: 0.001)
        XCTAssertEqual(grouped[1].intensity, 0.3, accuracy: 0.001)

        let fallback = IOSStreamHapticsRouter.pulses(
            for: effect,
            supportedRoutes: [.defaultLocality]
        )
        XCTAssertEqual(fallback.count, 1)
        XCTAssertEqual(fallback[0].route, .defaultLocality)
        XCTAssertEqual(fallback[0].intensity, 0.9, accuracy: 0.001)

        let minimumDuration = IOSStreamHapticsRouter.pulses(
            for: IOSStreamRumbleEffect(
                gamepadIndex: nil,
                strongMagnitude: 0.5,
                weakMagnitude: 0,
                leftTrigger: 0,
                rightTrigger: 0,
                durationMilliseconds: 1,
                format: .betterXcloud
            ),
            supportedRoutes: [.leftHandle]
        )
        XCTAssertEqual(minimumDuration.first?.durationMilliseconds, 10)

        let stop = IOSStreamRumbleEffect(
            gamepadIndex: 0,
            strongMagnitude: 1,
            weakMagnitude: 1,
            leftTrigger: 1,
            rightTrigger: 1,
            durationMilliseconds: 0,
            format: .betterXcloud
        )
        XCTAssertTrue(IOSStreamHapticsRouter.pulses(
            for: stop,
            supportedRoutes: Set(IOSStreamHapticsRoute.allCases)
        ).isEmpty)
    }

    func testIOSStreamHapticsTransitionStopsRoutesMissingFromNextEffect() {
        let nextPulse = IOSStreamHapticsPulse(
            route: .rightHandle,
            intensity: 0.5,
            sharpness: 0.8,
            durationMilliseconds: 100
        )

        let transition = IOSStreamHapticsTransitionPlanner.plan(
            activeRoutes: [.leftHandle, .rightHandle],
            nextPulses: [nextPulse]
        )

        XCTAssertEqual(transition.routesToStop, [.leftHandle])
        XCTAssertEqual(transition.pulsesToPlay, [nextPulse])
    }

    func testIOSStreamHapticsTransitionStopPacketAndUnavailableTargetStopAll() {
        let activeRoutes: Set<IOSStreamHapticsRoute> = [.leftHandle, .rightTrigger]
        let stopPacket = IOSStreamHapticsTransitionPlanner.plan(
            activeRoutes: activeRoutes,
            nextPulses: [],
            stopAll: true
        )
        let unavailableTarget = IOSStreamHapticsTransitionPlanner.plan(
            activeRoutes: activeRoutes,
            nextPulses: [
                IOSStreamHapticsPulse(
                    route: .rightHandle,
                    intensity: 1,
                    sharpness: 1,
                    durationMilliseconds: 100
                ),
            ],
            stopAll: true
        )

        XCTAssertEqual(stopPacket.routesToStop, activeRoutes)
        XCTAssertTrue(stopPacket.pulsesToPlay.isEmpty)
        XCTAssertEqual(unavailableTarget.routesToStop, activeRoutes)
        XCTAssertTrue(unavailableTarget.pulsesToPlay.isEmpty)
    }

    func testStreamingNativeIceCandidateAdapterNormalizesXboxCandidates() {
        let xcloudRelay = StreamingIceCandidate(
            sdp: "a=candidate:842163049 1 udp 1677734910 13.107.246.40 3478 "
                + "typ relay raddr 0.0.0.0 rport 0 generation 0 ufrag xcloud",
            sdpMid: "0",
            sdpMLineIndex: 0
        )
        let xhomeHost = StreamingIceCandidate(
            sdp: "a=candidate:1 1 UDP 2130706431 192.168.1.50 50000 typ host generation 0",
            sdpMid: "video",
            sdpMLineIndex: 1
        )
        let alreadyNative = StreamingIceCandidate(
            sdp: "candidate:2 1 udp 2122260223 10.0.0.8 60000 typ host",
            sdpMid: nil,
            sdpMLineIndex: 0
        )

        XCTAssertEqual(
            StreamingNativeIceCandidateAdapter.adapt(xcloudRelay),
            StreamingIceCandidate(
                sdp: "candidate:842163049 1 udp 1677734910 13.107.246.40 3478 "
                    + "typ relay raddr 0.0.0.0 rport 0 generation 0 ufrag xcloud",
                sdpMid: "0",
                sdpMLineIndex: 0
            )
        )
        XCTAssertEqual(
            StreamingNativeIceCandidateAdapter.adapt(xhomeHost),
            StreamingIceCandidate(
                sdp: "candidate:1 1 UDP 2130706431 192.168.1.50 50000 typ host generation 0",
                sdpMid: "video",
                sdpMLineIndex: 1
            )
        )
        XCTAssertEqual(StreamingNativeIceCandidateAdapter.adapt(alreadyNative), alreadyNative)
    }

    func testStreamingNativeIceCandidateAdapterDropsEmptyEOCAndInvalidUDPWithTCPType() {
        let rejected = [
            "",
            "   \n",
            "end-of-candidates",
            "a=end-of-candidates",
            "candidate:3 1 udp 2122260223 10.0.0.9 60001 typ host tcptype passive",
            "candidate:4 1 udp 2122260223 10.0.0.10 60002 typ host tcptype=active",
        ]

        for sdp in rejected {
            XCTAssertNil(StreamingNativeIceCandidateAdapter.adapt(StreamingIceCandidate(
                sdp: sdp,
                sdpMid: "0",
                sdpMLineIndex: 0
            )))
        }
    }

    func testStreamingDataChannelStateMachineRunsPreAndPostHandshakeStagesWithRetry() {
        var machine = StreamingDataChannelStateMachine(
            postHandshakeCount: 2,
            controlBootstrapCount: 2
        )
        machine.channelDidOpen(.input)

        XCTAssertEqual(machine.nextAction(), .sendInputMetadata(stage: .preHandshake))
        machine.actionDidFail(.sendInputMetadata(stage: .preHandshake))
        XCTAssertEqual(machine.nextAction(), .sendInputMetadata(stage: .preHandshake))
        machine.actionDidSucceed(.sendInputMetadata(stage: .preHandshake))

        machine.channelDidOpen(.control)
        XCTAssertEqual(
            machine.nextAction(),
            .sendControlBootstrap(stage: .preHandshake, index: 0)
        )
        machine.actionDidSucceed(.sendControlBootstrap(stage: .preHandshake, index: 0))
        XCTAssertEqual(
            machine.nextAction(),
            .sendControlBootstrap(stage: .preHandshake, index: 1)
        )
        machine.actionDidFail(.sendControlBootstrap(stage: .preHandshake, index: 1))
        XCTAssertEqual(
            machine.nextAction(),
            .sendControlBootstrap(stage: .preHandshake, index: 1)
        )
        machine.actionDidSucceed(.sendControlBootstrap(stage: .preHandshake, index: 1))

        machine.channelDidOpen(.chat)
        machine.channelDidOpen(.message)

        XCTAssertEqual(machine.nextAction(), .sendMessageHandshake)
        machine.actionDidFail(.sendMessageHandshake)
        XCTAssertEqual(machine.nextAction(), .sendMessageHandshake)
        machine.actionDidSucceed(.sendMessageHandshake)
        XCTAssertNil(machine.nextAction())

        let ack = Data(#"{"type":"HandshakeAck"}"#.utf8)
        XCTAssertNil(machine.receiveMessage(ack))
        XCTAssertEqual(machine.nextAction(), .sendPostHandshake(index: 0))
        machine.actionDidSucceed(.sendPostHandshake(index: 0))
        XCTAssertNil(machine.receiveMessage(ack))
        XCTAssertEqual(machine.nextAction(), .sendPostHandshake(index: 1))
        machine.actionDidFail(.sendPostHandshake(index: 1))
        XCTAssertEqual(machine.nextAction(), .sendPostHandshake(index: 1))
        machine.actionDidSucceed(.sendPostHandshake(index: 1))

        XCTAssertEqual(
            machine.nextAction(),
            .sendControlBootstrap(stage: .postHandshake, index: 0)
        )
        machine.actionDidSucceed(.sendControlBootstrap(stage: .postHandshake, index: 0))
        XCTAssertEqual(
            machine.nextAction(),
            .sendControlBootstrap(stage: .postHandshake, index: 1)
        )
        machine.actionDidSucceed(.sendControlBootstrap(stage: .postHandshake, index: 1))
        XCTAssertEqual(machine.nextAction(), .announceControlReady)
        machine.actionDidSucceed(.announceControlReady)
        XCTAssertEqual(machine.nextAction(), .sendInputMetadata(stage: .postHandshake))
        machine.actionDidFail(.sendInputMetadata(stage: .postHandshake))
        XCTAssertEqual(machine.nextAction(), .sendInputMetadata(stage: .postHandshake))
        machine.actionDidSucceed(.sendInputMetadata(stage: .postHandshake))
        XCTAssertEqual(machine.nextAction(), .scheduleGamepadAnnouncement)
        machine.actionDidSucceed(.scheduleGamepadAnnouncement)
        XCTAssertEqual(machine.nextAction(), .startInput)
        machine.actionDidSucceed(.startInput)
        XCTAssertNil(machine.nextAction())

        XCTAssertNil(machine.receiveMessage(ack))
        XCTAssertEqual(machine.snapshot.postHandshakeSentCount, 2)
        XCTAssertEqual(machine.snapshot.preHandshakeControlBootstrapSentCount, 2)
        XCTAssertEqual(machine.snapshot.postHandshakeControlBootstrapSentCount, 2)
        XCTAssertTrue(machine.snapshot.preHandshakeInputMetadataSent)
        XCTAssertTrue(machine.snapshot.postHandshakeInputMetadataSent)
        XCTAssertTrue(machine.snapshot.controlReady)
        XCTAssertTrue(machine.snapshot.inputStarted)
    }

    func testStreamingDataChannelStateMachineCoalescesLateOpenBootstrapAfterAck() {
        var machine = StreamingDataChannelStateMachine(
            postHandshakeCount: 0,
            controlBootstrapCount: 2
        )
        machine.channelDidOpen(.message)
        XCTAssertEqual(machine.nextAction(), .sendMessageHandshake)
        machine.actionDidSucceed(.sendMessageHandshake)

        let ack = Data(#"{"type":"HandshakeAck"}"#.utf8)
        XCTAssertNil(machine.receiveMessage(ack))
        XCTAssertNil(machine.nextAction())

        machine.channelDidOpen(.input)
        XCTAssertEqual(machine.nextAction(), .sendInputMetadata(stage: .postHandshake))
        machine.actionDidSucceed(.sendInputMetadata(stage: .postHandshake))
        XCTAssertNil(machine.nextAction())

        machine.channelDidOpen(.control)
        XCTAssertEqual(
            machine.nextAction(),
            .sendControlBootstrap(stage: .postHandshake, index: 0)
        )
        machine.actionDidFail(.sendControlBootstrap(stage: .postHandshake, index: 0))
        XCTAssertEqual(
            machine.nextAction(),
            .sendControlBootstrap(stage: .postHandshake, index: 0)
        )
        machine.actionDidSucceed(.sendControlBootstrap(stage: .postHandshake, index: 0))
        XCTAssertEqual(
            machine.nextAction(),
            .sendControlBootstrap(stage: .postHandshake, index: 1)
        )
        machine.actionDidSucceed(.sendControlBootstrap(stage: .postHandshake, index: 1))
        XCTAssertEqual(machine.nextAction(), .announceControlReady)
        machine.actionDidSucceed(.announceControlReady)
        XCTAssertEqual(machine.nextAction(), .scheduleGamepadAnnouncement)
        machine.actionDidSucceed(.scheduleGamepadAnnouncement)
        XCTAssertEqual(machine.nextAction(), .startInput)
        machine.actionDidSucceed(.startInput)

        XCTAssertNil(machine.receiveMessage(ack))
        XCTAssertNil(machine.nextAction())
        XCTAssertEqual(machine.snapshot.preHandshakeControlBootstrapSentCount, 2)
        XCTAssertEqual(machine.snapshot.postHandshakeControlBootstrapSentCount, 2)
        XCTAssertTrue(machine.snapshot.preHandshakeInputMetadataSent)
        XCTAssertTrue(machine.snapshot.postHandshakeInputMetadataSent)
        XCTAssertTrue(machine.snapshot.controlReady)
    }

    func testStreamingDataChannelStateMachineMessageRetryDoesNotBlockChannelPrefetch() {
        var machine = StreamingDataChannelStateMachine(
            postHandshakeCount: 0,
            controlBootstrapCount: 1
        )
        machine.channelDidOpen(.message)
        XCTAssertEqual(machine.nextAction(), .sendMessageHandshake)
        machine.actionDidFail(.sendMessageHandshake)

        machine.channelDidOpen(.control)
        XCTAssertEqual(
            machine.nextAction(),
            .sendControlBootstrap(stage: .preHandshake, index: 0)
        )
        machine.actionDidSucceed(.sendControlBootstrap(stage: .preHandshake, index: 0))
        XCTAssertEqual(machine.nextAction(), .sendMessageHandshake)
        machine.actionDidFail(.sendMessageHandshake)

        machine.channelDidOpen(.input)
        XCTAssertEqual(machine.nextAction(), .sendInputMetadata(stage: .preHandshake))
        machine.actionDidSucceed(.sendInputMetadata(stage: .preHandshake))
        XCTAssertEqual(machine.nextAction(), .sendMessageHandshake)
        machine.actionDidSucceed(.sendMessageHandshake)

        XCTAssertEqual(machine.snapshot.preHandshakeControlBootstrapSentCount, 1)
        XCTAssertTrue(machine.snapshot.preHandshakeInputMetadataSent)
        XCTAssertFalse(machine.snapshot.handshakeAcknowledged)
    }

    func testStreamingDataChannelStateMachineClassifiesRemoteTerminalOnce() {
        var machine = StreamingDataChannelStateMachine(
            postHandshakeCount: 0,
            controlBootstrapCount: 0
        )
        machine.channelDidOpen(.message)
        _ = machine.nextAction()
        machine.actionDidSucceed(.sendMessageHandshake)

        let kick = Data(#"{"type":"Message","content":"KickForClosedGame"}"#.utf8)
        XCTAssertEqual(machine.receiveMessage(kick), .remoteKickClosedGame)
        XCTAssertNil(machine.receiveMessage(Data(#"{"type":"Error"}"#.utf8)))
        XCTAssertEqual(machine.snapshot.terminalReason, .remoteKickClosedGame)
        XCTAssertNil(machine.nextAction())

        var closed = StreamingDataChannelStateMachine(
            postHandshakeCount: 0,
            controlBootstrapCount: 0
        )
        XCTAssertNil(closed.channelDidClose(.chat))
        closed.channelDidOpen(.chat)
        XCTAssertEqual(closed.snapshot.phases[.chat], .open)
        XCTAssertEqual(closed.channelDidClose(.control), .dataChannelClosed(.control))
        XCTAssertNil(closed.channelDidClose(.input))
        closed.channelDidOpen(.control)
        XCTAssertEqual(closed.snapshot.phases[.control], .closed)
    }

    func testStreamingRemoteMessageClassifierHandlesNestedCloseAndErrorPayloads() {
        XCTAssertEqual(
            StreamingRemoteMessageClassifier.terminalReason(
                Data(#"{"type":"Message","content":"{\"reason\":\"sessionClosed\"}"}"#.utf8)
            ),
            .remoteClosed
        )
        XCTAssertEqual(
            StreamingRemoteMessageClassifier.terminalReason(
                Data(#"{"type":"Error","error":{"code":"TransportError"}}"#.utf8)
            ),
            .remoteError
        )
        XCTAssertNil(
            StreamingRemoteMessageClassifier.terminalReason(
                Data(#"{"type":"Message","target":"/streaming/characteristics/dimensionschanged"}"#.utf8)
            )
        )
    }

    func testGameSummaryKeepsUnavailablePlaytimeDistinct() {
        let game = GameSummary(
            id: "halo-infinite",
            titleID: "1292135258",
            name: "Halo Infinite",
            artworkURL: nil,
            playtimeMinutes: nil,
            achievementProgress: nil
        )

        XCTAssertNil(game.playtimeMinutes)
    }

    func testXboxPresentationFormatsPlaytime() {
        XCTAssertEqual(XboxPresentation.playtime(nil), "时长未知")
        XCTAssertEqual(XboxPresentation.playtime(45), "45 分钟")
        XCTAssertEqual(XboxPresentation.playtime(90), "1.5 小时")
        XCTAssertEqual(XboxPresentation.playtime(725), "12 小时")
    }

    func testXboxImageURLNormalizesSecureSchemes() {
        XCTAssertEqual(
            XboxImageURL.resolve("http://images.example.com/game.jpg")?.absoluteString,
            "https://images.example.com/game.jpg"
        )
        XCTAssertEqual(
            XboxImageURL.resolve("//images.example.com/game.jpg")?.absoluteString,
            "https://images.example.com/game.jpg"
        )
        XCTAssertEqual(
            XboxImageURL.resolve("https://images.example.com/game.jpg")?.absoluteString,
            "https://images.example.com/game.jpg"
        )
    }

    func testLibraryPresentationSortsRecentAndKeepsNilDatesInSourceOrder() {
        let older = Date(timeIntervalSince1970: 1_000)
        let newer = Date(timeIntervalSince1970: 2_000)
        let games = [
            makeGame(id: "nil-zulu", name: "Zulu", lastPlayedAt: nil),
            makeGame(id: "same-zulu", name: "Zulu", lastPlayedAt: older),
            makeGame(id: "newer", name: "Newest", lastPlayedAt: newer),
            makeGame(id: "same-alpha", name: "Alpha", lastPlayedAt: older),
            makeGame(id: "nil-alpha", name: "Alpha", lastPlayedAt: nil),
        ]

        let recent = LibraryPresentation.collections(from: games)
            .first { $0.kind == .recent }

        XCTAssertEqual(
            recent?.games.map(\.id),
            ["newer", "same-alpha", "same-zulu"]
        )
    }

    func testLibraryPresentationMatchesDesktopXcloudDimensions() {
        let games = [
            makeCloudGame(id: "recent", isRecentlyPlayed: true),
            makeCloudGame(id: "new", isNew: true),
            makeCloudGame(
                id: "activity-only",
                lastPlayedAt: Date(timeIntervalSince1970: 2_000)
            ),
            makeCloudGame(id: "all"),
        ]

        let collections = LibraryPresentation.collections(fromCloudGames: games)

        XCTAssertEqual(collections.map(\.kind), [.recent, .newlyAdded, .all])
        XCTAssertEqual(collections.map(\.title), ["最近游玩", "新入库", "全部云游戏"])
        XCTAssertEqual(collections.first { $0.kind == .recent }?.games.map(\.id), ["recent"])
    }

    func testLibraryPresentationKeepsCloudSectionSourceOrder() {
        let games = [
            makeCloudGame(id: "new-b", isNew: true),
            makeCloudGame(
                id: "recent-b",
                isRecentlyPlayed: true,
                lastPlayedAt: Date(timeIntervalSince1970: 1_000)
            ),
            makeCloudGame(
                id: "recent-a",
                isRecentlyPlayed: true,
                lastPlayedAt: Date(timeIntervalSince1970: 5_000)
            ),
            makeCloudGame(id: "new-a", isNew: true),
            makeCloudGame(id: "all"),
        ]

        let collections = LibraryPresentation.collections(fromCloudGames: games)

        XCTAssertEqual(
            collections.first { $0.kind == .recent }?.games.map(\.id),
            ["recent-b", "recent-a"]
        )
        XCTAssertEqual(
            collections.first { $0.kind == .newlyAdded }?.games.map(\.id),
            ["new-b", "new-a"]
        )
    }

    func testCloudLibraryStoreUsesDesktopCatalogLocaleContract() {
        XCTAssertEqual(
            CloudLibraryStore.catalogLanguage(preferredLanguage: "zh-Hans-CN"),
            "zh-TW"
        )
        XCTAssertEqual(
            CloudLibraryStore.catalogLanguage(preferredLanguage: "en-US"),
            "en-US"
        )
        XCTAssertEqual(CloudLibraryStore.catalogMarket(), "US")
    }

    func testLibraryPresentationTruncatesHomeAndHeroWithoutTruncatingCollection() {
        let games = (0..<10).map { index in
            makeGame(
                id: "game-\(index)",
                name: "Game \(index)",
                lastPlayedAt: Date(timeIntervalSince1970: TimeInterval(index))
            )
        }

        let recent = LibraryPresentation.collections(from: games)
            .first { $0.kind == .recent }

        XCTAssertEqual(recent?.games.count, 10)
        XCTAssertEqual(recent?.homeGames.count, 8)
        XCTAssertEqual(recent?.homeGames.map(\.id), Array(recent?.games.prefix(8).map(\.id) ?? []))
        XCTAssertEqual(LibraryPresentation.heroGames(from: games).map(\.id), [
            "game-9", "game-8", "game-7", "game-6", "game-5",
        ])
    }

    func testLibraryPresentationKeepsAllGamesAndHidesEmptyOptionalCollections() {
        let duplicateFirst = makeGame(id: "duplicate-first", name: "Same")
        let duplicateSecond = makeGame(id: "duplicate-second", name: "Same")
        let games = [
            makeGame(id: "zulu", name: "Zulu"),
            duplicateFirst,
            makeGame(id: "alpha", name: "Alpha"),
            duplicateSecond,
        ]

        let collections = LibraryPresentation.collections(from: games)
        let all = collections.first { $0.kind == .all }

        XCTAssertEqual(collections.map(\.kind), [.all])
        XCTAssertEqual(
            all?.games.map(\.id),
            ["alpha", "duplicate-first", "duplicate-second", "zulu"]
        )
        XCTAssertEqual(all?.games.count, games.count)
        XCTAssertEqual(LibraryPresentation.collections(from: []), [])
    }

    func testLibraryPresentationMetadataUsesCollectionDimension() {
        let game = makeCloudGame(
            id: "metadata",
            isNew: true
        )

        XCTAssertEqual(
            LibraryPresentation.metadata(for: game, kind: .newlyAdded),
            "Game Pass 新入库"
        )
        XCTAssertEqual(
            LibraryPresentation.metadata(
                for: makeGame(id: "fallback", name: "Fallback"),
                kind: .all
            ),
            "Xbox 游戏"
        )
    }

    func testStoredAuthSessionRoundTrip() throws {
        let session = StoredAuthSession(
            refreshToken: "refresh",
            seedJSON: "{\"seed\":true}",
            webTokenJSON: "{\"token\":true}",
            appLevel: 2
        )

        let encoded = try JSONEncoder().encode(session)
        XCTAssertEqual(try JSONDecoder().decode(StoredAuthSession.self, from: encoded), session)

        let encodedObject = try XCTUnwrap(String(data: encoded, encoding: .utf8))
        XCTAssertFalse(encodedObject.contains("accessHandle"))
        XCTAssertFalse(encodedObject.contains("gsToken"))
        XCTAssertFalse(encodedObject.contains("transferToken"))
        XCTAssertFalse(encodedObject.contains("sessionId"))
    }

    @MainActor
    func testAppSettingsStorePersistsCloudRegionPreset() {
        let suiteName = "XBXRC.SettingsTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let store = AppSettingsStore(defaults: defaults)
        XCTAssertEqual(store.cloudRegionPreset, .default)
        XCTAssertTrue(store.setCloudRegionPreset(.japan))
        XCTAssertEqual(AppSettingsStore(defaults: defaults).cloudRegionPreset, .japan)
        XCTAssertEqual(store.cloudRegionPreset.forceRegionIP, "210.131.113.123")

        store.usesEphemeralLoginSession = true
        XCTAssertTrue(AppSettingsStore(defaults: defaults).usesEphemeralLoginSession)
    }

    @MainActor
    func testAppSettingsStorePersistsAppearanceAndIconPreset() {
        let suiteName = "XBXRC.AppearanceSettingsTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let store = AppSettingsStore(defaults: defaults)
        XCTAssertEqual(store.appearanceMode, .system)
        XCTAssertEqual(store.appIconPreset, .default)

        store.appearanceMode = .dark
        defaults.set(AppIconPreset.forest.rawValue, forKey: AppSettingsStore.appIconPresetKey)

        let restored = AppSettingsStore(defaults: defaults)
        XCTAssertEqual(restored.appearanceMode, .dark)
        XCTAssertEqual(restored.appIconPreset, .forest)
        XCTAssertEqual(AppIconPreset.forest.alternateIconName, "AppIconForest")
        XCTAssertEqual(AppIconPreset.midnight.alternateIconName, "AppIconMidnight")
    }

    @MainActor
    func testAppSettingsStorePersistsConsumedStreamingSessionSettings() {
        let suiteName = "XBXRC.StreamingSettingsTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let store = AppSettingsStore(defaults: defaults)
        store.preferredGameLocale = "ja-JP"
        store.cloudResolution = .p1440
        store.homeResolution = .p720
        store.preferIPv6 = true
        store.codecPreference = .h264Main
        store.homeBitrateMode = .custom
        store.homeBitrateMbps = 35
        store.cloudBitrateMode = .custom
        store.cloudBitrateMbps = 18
        store.audioBitrateMode = .custom
        store.audioBitrateKbps = 192
        store.homeTurnFallbackEnabled = false

        let restored = AppSettingsStore(defaults: defaults)
        XCTAssertEqual(restored.preferredGameLocale, "ja-JP")
        XCTAssertEqual(restored.cloudResolution, .p1440)
        XCTAssertEqual(restored.homeResolution, .p720)
        XCTAssertTrue(restored.preferIPv6)
        XCTAssertEqual(restored.codecPreference, .h264Main)
        XCTAssertEqual(restored.homeBitrateMode, .custom)
        XCTAssertEqual(restored.homeBitrateMbps, 35)
        XCTAssertEqual(restored.cloudBitrateMode, .custom)
        XCTAssertEqual(restored.cloudBitrateMbps, 18)
        XCTAssertEqual(restored.audioBitrateMode, .custom)
        XCTAssertEqual(restored.audioBitrateKbps, 192)
        XCTAssertFalse(restored.homeTurnFallbackEnabled)

        let snapshot = restored.streamingSessionSettings
        XCTAssertEqual(snapshot.preferredGameLocale, "ja-JP")
        XCTAssertEqual(snapshot.cloudResolution, 1440)
        XCTAssertEqual(snapshot.homeResolution, 720)
        XCTAssertTrue(snapshot.preferIPv6)
        XCTAssertEqual(snapshot.videoCodec, "video/H264-4d")
        XCTAssertEqual(snapshot.homeBitrateMode, "Custom")
        XCTAssertEqual(snapshot.homeBitrateMbps, 35)
        XCTAssertEqual(snapshot.cloudBitrateMbps, 18)
        XCTAssertEqual(snapshot.audioBitrateKbps, 192)
        XCTAssertFalse(snapshot.homeTurnFallback)
    }

    func testMySettingsPresentationDerivesAccountAndSettingsSummaries() {
        let accessStatuses: [(appLevel: Int?, expected: String)] = [
            (nil, "未登录"),
            (0, "等待刷新"),
            (1, "地区受限"),
            (2, "可用"),
            (3, "可用"),
        ]
        for value in accessStatuses {
            XCTAssertEqual(
                MySettingsPresentation.cloudAccessStatus(for: value.appLevel),
                value.expected
            )
        }

        let presentation = MySettingsPresentation(
            appLevel: 2,
            cloudRegionTitle: "日本",
            usesEphemeralLoginSession: true,
            traceProfileTitle: "详细",
            version: "1.4.0 (140)"
        )

        XCTAssertEqual(presentation.cloudAccessStatus, "可用")
        XCTAssertEqual(presentation.cloudGamingSummary, "日本 · 可用")
        XCTAssertEqual(presentation.loginMode, "无 Cookie 临时会话")
        XCTAssertEqual(presentation.traceSummary, "详细")
        XCTAssertEqual(presentation.version, "XBXRC 1.4.0 (140)")

        let signedOut = MySettingsPresentation(
            appLevel: nil,
            cloudRegionTitle: "默认",
            usesEphemeralLoginSession: false,
            traceProfileTitle: "标准",
            version: "1.4.0"
        )
        XCTAssertEqual(signedOut.cloudGamingSummary, "默认 · 未登录")
        XCTAssertEqual(signedOut.loginMode, "标准会话")
    }

    func testCloudLibraryImageCandidatesPreferSuccessfulImageAndKeepFallbackOrder() {
        let game = makeCloudGame(
            id: "halo",
            heroURL: URL(string: "https://example.invalid/hero.jpg"),
            posterURL: URL(string: "https://example.invalid/poster.jpg"),
            tileURL: URL(string: "https://example.invalid/tile.jpg"),
            artworkURL: URL(string: "https://example.invalid/artwork.jpg")
        )
        let preferred = URL(string: "https://example.invalid/success.jpg")!

        XCTAssertEqual(
            game.imageCandidates(preferredURL: preferred).map(\.absoluteString),
            [
                preferred.absoluteString,
                "https://example.invalid/hero.jpg",
                "https://example.invalid/poster.jpg",
                "https://example.invalid/tile.jpg",
                "https://example.invalid/artwork.jpg",
            ]
        )
    }

    func testCloudCatalogSnapshotFreshStaleAndExpiredWindows() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let scope = CloudCatalogScope(
            accountID: "xid",
            regionHost: "region.example.com",
            language: "zh-CN",
            market: "CN"
        )

        let fresh = CloudCatalogSnapshot(
            scope: scope,
            games: [],
            baseUpdatedAt: now.addingTimeInterval(-60),
            overlayUpdatedAt: now.addingTimeInterval(-60)
        )
        let stale = CloudCatalogSnapshot(
            scope: scope,
            games: [],
            baseUpdatedAt: now.addingTimeInterval(-3_600),
            overlayUpdatedAt: now.addingTimeInterval(-601)
        )
        let expired = CloudCatalogSnapshot(
            scope: scope,
            games: [],
            baseUpdatedAt: now.addingTimeInterval(-3_600),
            overlayUpdatedAt: now.addingTimeInterval(-(24 * 60 * 60 + 1))
        )

        XCTAssertEqual(fresh.cacheState(at: now), .fresh)
        XCTAssertEqual(stale.cacheState(at: now), .stale)
        XCTAssertEqual(expired.cacheState(at: now), .expired)
    }

    func testCloudLibraryDiagnosticsRedactsSensitiveErrorDetails() {
        let error = NSError(
            domain: "CloudTest",
            code: 42,
            userInfo: [
                NSLocalizedDescriptionKey:
                    "request https://example.invalid/path?token=secret Bearer abc123 cloud-0123456789abcdef"
            ]
        )

        let summary = CloudLibraryDiagnostics.safeError(error)

        XCTAssertTrue(summary.contains("CloudTest#42"))
        XCTAssertFalse(summary.contains("https://"))
        XCTAssertFalse(summary.contains("abc123"))
        XCTAssertFalse(summary.contains("cloud-0123456789abcdef"))
    }

    func testCloudLibraryDiagnosticsProjectsStreamingOfferingFailure() {
        let error = NSError(
            domain: "XboxBridgeError",
            code: 1,
            userInfo: [
                NSLocalizedDescriptionKey:
                    "xCloud token is unavailable; forceRegionApplied=false; "
                    + "offering=xgpuweb,errorKind=http,statusCode=403,timeout=false,retriable=false; "
                    + "offering=xgpuwebf2p,errorKind=network,statusCode=none,timeout=true,retriable=true"
            ]
        )

        let payload = CloudLibraryDiagnostics.errorPayload(error)

        XCTAssertEqual(payload["errorKind"], .string("xgpuwebf2p"))
        XCTAssertEqual(payload["statusCode"], .integer(403))
        XCTAssertEqual(payload["timeout"], .bool(true))
        XCTAssertEqual(payload["retriable"], .bool(true))
        XCTAssertEqual(payload["offerings"], .string("xgpuweb,xgpuwebf2p"))
        XCTAssertEqual(payload["forceRegionApplied"], .bool(false))
    }

    func testIOSRuntimeTraceWriterEmitsSchemaV3AndRedactsPayload() async throws {
        let rootDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: rootDirectory) }
        let writer = IOSRuntimeTraceWriter(
            rootDirectory: rootDirectory,
            profile: .dev,
            launchSessionID: "launch-session"
        )

        writer.record(
            IOSRuntimeTraceDraft(
                category: .event,
                domain: "cloud-library",
                event: "catalogRefreshStarted",
                payload: [
                    "generation": .integer(3),
                    "refreshToken": .string("secret-refresh-token"),
                    "hasRefreshToken": .bool(true),
                    "accountID": .string("123456789"),
                    "message": .string("request https://example.invalid/path?token=secret"),
                ],
                dimension: .network,
                importance: .key,
                operationID: "operation-1"
            )
        )
        writer.record(
            IOSRuntimeTraceDraft(
                category: .state,
                domain: "cloud-library",
                event: "catalogRefreshCommitted",
                payload: ["games": .integer(1_888)],
                dimension: .core,
                importance: .essential,
                operationID: "operation-1"
            )
        )

        await writer.flush()
        let envelopes = try await traceEnvelopes(from: writer)
        let events = envelopes.filter { $0.domain == "cloud-library" }

        XCTAssertEqual(events.count, 2)
        XCTAssertEqual(events.map(\.seq), events.map(\.seq).sorted())
        XCTAssertTrue(events.allSatisfy { $0.schemaVersion == 3 })
        XCTAssertTrue(events.allSatisfy { $0.sessionId == "launch-session" })
        XCTAssertEqual(events.first?.payload["refreshToken"], .string("<redacted>"))
        XCTAssertEqual(events.first?.payload["hasRefreshToken"], .bool(true))
        XCTAssertEqual(events.first?.payload["accountID"], .string("<redacted>"))
        XCTAssertEqual(events.first?.payload["message"], .string("request <url>"))
        XCTAssertEqual(events.first?.payload["operationId"], .string("operation-1"))
        XCTAssertEqual(events.first?.payload["platform"], .string("ios"))
    }

    func testIOSRuntimeTraceProductionFiltersDebugRows() async throws {
        let rootDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: rootDirectory) }
        let writer = IOSRuntimeTraceWriter(
            rootDirectory: rootDirectory,
            profile: .production
        )

        writer.record(
            IOSRuntimeTraceDraft(
                category: .event,
                domain: "test",
                event: "debugRow",
                payload: [:],
                dimension: .core,
                importance: .debug,
                operationID: nil
            )
        )
        writer.record(
            IOSRuntimeTraceDraft(
                category: .state,
                domain: "test",
                event: "keyRow",
                payload: [:],
                dimension: .core,
                importance: .key,
                operationID: nil
            )
        )

        await writer.flush()
        let envelopes = try await traceEnvelopes(from: writer)

        XCTAssertFalse(envelopes.contains { $0.event == "debugRow" })
        XCTAssertTrue(envelopes.contains { $0.event == "keyRow" })
    }

    func testIOSRuntimeTraceProfileBudgetsStayBounded() {
        XCTAssertEqual(
            IOSRuntimeTracePolicy.budget(for: .production),
            IOSRuntimeTraceBudget(maxFileBytes: 8 * 1_024 * 1_024, maxFiles: 4)
        )
        XCTAssertEqual(
            IOSRuntimeTracePolicy.budget(for: .dev),
            IOSRuntimeTraceBudget(maxFileBytes: 32 * 1_024 * 1_024, maxFiles: 6)
        )
        XCTAssertEqual(
            IOSRuntimeTracePolicy.budget(for: .off),
            IOSRuntimeTraceBudget(maxFileBytes: 0, maxFiles: 0)
        )
    }

    func testIOSRuntimeTraceRotatesAndPrunesFiles() async throws {
        let rootDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: rootDirectory) }
        let writer = IOSRuntimeTraceWriter(
            rootDirectory: rootDirectory,
            profile: .dev,
            budgetOverride: IOSRuntimeTraceBudget(maxFileBytes: 900, maxFiles: 2)
        )

        for index in 0..<24 {
            writer.record(
                IOSRuntimeTraceDraft(
                    category: .snapshot,
                    domain: "rotation-test",
                    event: "sample",
                    payload: [
                        "index": .integer(Int64(index)),
                        "content": .string(String(repeating: "x", count: 160)),
                    ],
                    dimension: .core,
                    importance: .key,
                    operationID: nil
                )
            )
        }

        await writer.flush()
        let files = await writer.traceFiles()
        let envelopes = try await traceEnvelopes(from: writer)
        let fileSizes = try files.map { file in
            try XCTUnwrap(
                file.resourceValues(forKeys: [.fileSizeKey]).fileSize
            )
        }

        XCTAssertEqual(files.count, 2)
        XCTAssertTrue(fileSizes.allSatisfy { $0 <= 900 })
        XCTAssertFalse(envelopes.isEmpty)
        XCTAssertEqual(envelopes.map(\.seq), envelopes.map(\.seq).sorted())
        XCTAssertEqual(Set(envelopes.map(\.seq)).count, envelopes.count)
        XCTAssertTrue(envelopes.contains { $0.event == "fileOpened" })
    }

    func testCloudCatalogSnapshotRepositoryRoundTripsBaseAndOverlay() async throws {
        let rootDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: rootDirectory) }

        let scope = CloudCatalogScope(
            accountID: "xid",
            regionHost: "region.example.com",
            language: "zh-CN",
            market: "CN"
        )
        let baseUpdatedAt = Date(timeIntervalSince1970: 1_000_000)
        let overlayUpdatedAt = Date(timeIntervalSince1970: 1_000_500)
        let preferredImageURL = URL(string: "https://example.invalid/success.jpg")!
        let game = CloudLibraryGame(
            productID: "P123",
            streamTitleID: "stream-P123",
            xboxTitleID: "123",
            name: "Forza Horizon",
            publisherName: "Xbox Game Studios",
            description: "Open-world racing",
            tileURL: URL(string: "https://example.invalid/tile.jpg"),
            posterURL: URL(string: "https://example.invalid/poster.jpg"),
            heroURL: URL(string: "https://example.invalid/hero.jpg"),
            artworkURL: URL(string: "https://example.invalid/artwork.jpg"),
            categories: ["Racing", "Open World"],
            supportedInputTypes: ["Controller", "Touch"],
            hasEntitlement: true,
            isRecentlyPlayed: true,
            isNew: false,
            lastPlayedAt: Date(timeIntervalSince1970: 999_900),
            playtimeMinutes: 725,
            achievementProgress: AchievementProgress(
                unlockedCount: 12,
                totalCount: 50,
                earnedGamerscore: 240,
                totalGamerscore: 1_000,
                percentage: 24
            )
        )
        let snapshot = CloudCatalogSnapshot(
            scope: scope,
            games: [game],
            baseUpdatedAt: baseUpdatedAt,
            overlayUpdatedAt: overlayUpdatedAt,
            successfulImageURLs: [game.productID: preferredImageURL]
        )
        let repository = CloudCatalogSnapshotRepository(rootDirectory: rootDirectory)

        try await repository.save(snapshot)
        let restored = try await repository.load(scope: scope)

        XCTAssertEqual(restored, snapshot)
        XCTAssertEqual(restored?.cacheState(at: overlayUpdatedAt), .fresh)

        let filenames = try FileManager.default.contentsOfDirectory(
            at: rootDirectory,
            includingPropertiesForKeys: nil
        ).map(\.lastPathComponent)
        XCTAssertEqual(filenames.filter { $0.hasPrefix("base-v1-") }.count, 1)
        XCTAssertEqual(filenames.filter { $0.hasPrefix("overlay-v1-") }.count, 1)
    }

    func testCloudCatalogSnapshotRepositoryClearsAccountOverlayAndKeepsBase() async throws {
        let rootDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: rootDirectory) }

        let scope = CloudCatalogScope(
            accountID: "xid",
            regionHost: "region.example.com",
            language: "zh-CN",
            market: "CN"
        )
        let snapshot = CloudCatalogSnapshot(
            scope: scope,
            games: [makeCloudGame(id: "P123")],
            baseUpdatedAt: .now,
            overlayUpdatedAt: .now
        )
        let repository = CloudCatalogSnapshotRepository(rootDirectory: rootDirectory)

        try await repository.save(snapshot)
        try await repository.clearOverlay(accountID: scope.accountID)

        let restored = try await repository.load(scope: scope)
        XCTAssertNil(restored)
        let filenames = try FileManager.default.contentsOfDirectory(
            at: rootDirectory,
            includingPropertiesForKeys: nil
        ).map(\.lastPathComponent)
        XCTAssertEqual(filenames.filter { $0.hasPrefix("base-v1-") }.count, 1)
        XCTAssertTrue(filenames.allSatisfy { !$0.hasPrefix("overlay-v1-") })
    }

    func testLibraryPresentationKeepsLargeCatalogProgressiveWindowsBounded() {
        let games = (0..<1_000).map { index in
            makeCloudGame(
                id: String(format: "P%04d", index),
                isRecentlyPlayed: index < 25,
                isNew: index < 50
            )
        }
        let collections = LibraryPresentation.collections(fromCloudGames: games)

        XCTAssertEqual(collections.first { $0.kind == .recent }?.games.count, 25)
        XCTAssertEqual(collections.first { $0.kind == .newlyAdded }?.games.count, 50)
        XCTAssertEqual(collections.first { $0.kind == .all }?.games.count, 1_000)
        XCTAssertEqual(collections.first { $0.kind == .all }?.homeGames.count, 8)
        XCTAssertEqual(LibraryPresentation.heroGames(fromCloudGames: games).count, 5)
        XCTAssertEqual(LibraryLayoutMetrics.collectionPageSize, 24)
    }

    @MainActor
    func testCloudLibraryStoreCoalescesConcurrentRefreshes() async {
        let scope = CloudCatalogScope(
            accountID: "xid",
            regionHost: "region.example.com",
            language: "zh-CN",
            market: "CN"
        )
        let client = MockXboxCloudDataClient(
            snapshot: RemoteCloudCatalogSnapshot(
                games: (0..<1_000).map { makeCloudGame(id: "P\($0)") },
                scope: scope,
                fetchedAt: .now,
                failedHydrationChunks: 0,
                pendingHydrationProductIDs: []
            )
        )
        let store = CloudLibraryStore(
            client: client,
            repository: InMemoryCloudCatalogSnapshotStore()
        )
        let access = PreparedCloudAccess(
            authSession: AuthSession(
                refreshToken: "refresh",
                seedJson: "seed",
                webTokenJson: "web",
                appLevel: 2
            ),
            handle: "handle",
            accountID: "xid",
            regionHost: "region.example.com"
        )

        async let first: Void = store.refresh(reason: .pullToRefresh) { access }
        async let second: Void = store.refresh(reason: .manualRetry) { access }
        async let third: Void = store.refresh(reason: .pageEnter) { access }
        _ = await (first, second, third)

        let requestCount = await client.catalogRequestCount()
        XCTAssertEqual(requestCount, 1)
        XCTAssertEqual(store.games.count, 1_000)
        XCTAssertEqual(store.phase, .loaded)
    }

    @MainActor
    func testCloudLibraryStoreActivatesCachedScopeOnceThenRefreshesManually() async {
        let scope = CloudCatalogScope(
            accountID: "xid",
            regionHost: "region.example.com",
            language: "zh-CN",
            market: "CN"
        )
        let cachedGame = makeCloudGame(id: "cached")
        let remoteGame = makeCloudGame(id: "remote")
        let repository = InMemoryCloudCatalogSnapshotStore(
            snapshot: CloudCatalogSnapshot(
                scope: scope,
                games: [cachedGame],
                baseUpdatedAt: .now,
                overlayUpdatedAt: .now
            )
        )
        let client = MockXboxCloudDataClient(
            snapshot: RemoteCloudCatalogSnapshot(
                games: [remoteGame],
                scope: scope,
                fetchedAt: .now,
                failedHydrationChunks: 0,
                pendingHydrationProductIDs: []
            )
        )
        let store = CloudLibraryStore(client: client, repository: repository)
        let session = StoredAuthSession(
            refreshToken: "refresh",
            seedJSON: "seed",
            webTokenJSON: "web",
            appLevel: 2,
            cloudAccountID: scope.accountID,
            cloudRegionHost: scope.regionHost
        )
        let access = PreparedCloudAccess(
            authSession: AuthSession(
                refreshToken: "refresh",
                seedJson: "seed",
                webTokenJson: "web",
                appLevel: 2
            ),
            handle: "handle",
            accountID: scope.accountID,
            regionHost: scope.regionHost
        )

        await store.restoreCached(session: session)
        XCTAssertEqual(store.games, [cachedGame])

        await store.activateOnce(session: session) { access }
        await store.activateOnce(session: session) { access }
        let initialRequestCount = await client.catalogRequestCount()
        XCTAssertEqual(initialRequestCount, 1)
        XCTAssertEqual(store.games, [remoteGame])

        async let first: Void = store.refresh(reason: .pullToRefresh) { access }
        async let second: Void = store.refresh(reason: .manualRetry) { access }
        _ = await (first, second)
        let manualRequestCount = await client.catalogRequestCount()
        XCTAssertEqual(manualRequestCount, 2)
    }

    @MainActor
    func testRestoreWithoutStoredSessionShowsSignedOutState() async {
        let store = AuthStore(
            client: MockXboxAuthClient(),
            keychain: InMemoryAuthSessionStore(),
            webAuthentication: MockWebAuthentication()
        )

        await store.restore()

        XCTAssertEqual(store.phase, .signedOut)
        XCTAssertFalse(store.isSignedIn)
    }

    @MainActor
    func testRestoreRenewsCredentialsWithoutLoadingProfile() async {
        let original = StoredAuthSession(
            refreshToken: "old-refresh",
            seedJSON: "seed",
            webTokenJSON: "old-web-token",
            appLevel: 1
        )
        let keychain = InMemoryAuthSessionStore(session: original)
        let recorder = RegionRoutingRecorder()
        let client = MockXboxAuthClient(
            renewedSession: AuthSession(
                refreshToken: "new-refresh",
                seedJson: "seed",
                webTokenJson: "new-web-token",
                appLevel: 2
            ),
            profile: Self.profile,
            recorder: recorder
        )
        let store = AuthStore(
            client: client,
            keychain: keychain,
            webAuthentication: MockWebAuthentication()
        )

        await store.restore()

        let savedSession = await keychain.currentSession()

        XCTAssertEqual(store.phase, .signedIn)
        XCTAssertNil(store.profile)
        XCTAssertEqual(store.session?.refreshToken, "new-refresh")
        XCTAssertEqual(savedSession?.refreshToken, "new-refresh")
        let profileRequestCount = await recorder.profileRequestCount()
        XCTAssertEqual(profileRequestCount, 0)
    }

    @MainActor
    func testRestoreUsesConfiguredCloudRegionForSessionRenewal() async {
        let recorder = RegionRoutingRecorder()
        let client = MockXboxAuthClient(recorder: recorder)
        let settings = MockCloudRegionSettings(preset: .japan)
        let store = AuthStore(
            settings: settings,
            client: client,
            keychain: InMemoryAuthSessionStore(session: Self.storedSession),
            webAuthentication: MockWebAuthentication()
        )

        await store.restore()

        let renewRegionIP = await recorder.lastRenewRegionIP()
        XCTAssertEqual(renewRegionIP, "210.131.113.123")
    }

    @MainActor
    func testInteractiveLoginPersistsSessionAndProfile() async {
        let keychain = InMemoryAuthSessionStore()
        let recorder = RegionRoutingRecorder()
        let client = MockXboxAuthClient(
            finishedSession: AuthSession(
                refreshToken: "refresh",
                seedJson: "seed",
                webTokenJson: "web-token",
                appLevel: 2
            ),
            profile: Self.profile,
            recorder: recorder
        )
        let webAuthentication = MockWebAuthentication(
            callbackURL: URL(string: "ms-xal-000000004c20a908://auth/?code=code")!
        )
        let store = AuthStore(
            settings: MockCloudRegionSettings(
                preset: .default,
                usesEphemeralLoginSession: true
            ),
            client: client,
            keychain: keychain,
            webAuthentication: webAuthentication
        )

        await store.restore()
        await store.signIn()
        await store.activateProfileOnce()
        await store.activateProfileOnce()

        let savedSession = await keychain.currentSession()

        XCTAssertEqual(store.phase, .signedIn)
        XCTAssertEqual(store.profile, Self.profile)
        XCTAssertEqual(savedSession?.webTokenJSON, "web-token")
        XCTAssertEqual(webAuthentication.lastPrefersEphemeralSession, true)
        let profileRequestCount = await recorder.profileRequestCount()
        XCTAssertEqual(profileRequestCount, 1)
    }

    @MainActor
    func testXboxDataStoreMergesPlaytimeIntoGameLibrary() async {
        let store = XboxDataStore(
            client: MockXboxDataClient(
                games: [Self.game],
                playtimes: [TitlePlaytime(titleID: Self.game.titleID, minutes: 725)]
            )
        )

        await store.sync(session: Self.storedSession)
        await store.activateLibraryOnce()

        XCTAssertEqual(store.libraryPhase, .loaded)
        XCTAssertEqual(store.games.count, 1)
        XCTAssertEqual(store.games.first?.playtimeMinutes, 725)
        XCTAssertNil(store.libraryErrorMessage)
    }

    @MainActor
    func testXboxDataStoreBindsCredentialsLazilyAndCoalescesRefreshes() async {
        let recorder = XboxDataClientRecorder()
        let store = XboxDataStore(
            client: MockXboxDataClient(
                games: [Self.game],
                recorder: recorder,
                libraryDelay: .milliseconds(40)
            )
        )

        await store.sync(session: Self.storedSession, ownerGeneration: 7)
        let initialRequestCount = await recorder.libraryRequestCount()
        XCTAssertEqual(initialRequestCount, 0)

        let renewed = StoredAuthSession(
            refreshToken: "renewed-refresh",
            seedJSON: Self.storedSession.seedJSON,
            webTokenJSON: "renewed-web-token",
            appLevel: 1
        )
        await store.sync(session: renewed, ownerGeneration: 7)
        let renewedRequestCount = await recorder.libraryRequestCount()
        XCTAssertEqual(renewedRequestCount, 0)

        async let firstActivation: Void = store.activateLibraryOnce()
        async let duplicateActivation: Void = store.activateLibraryOnce()
        _ = await (firstActivation, duplicateActivation)
        let activationRequestCount = await recorder.libraryRequestCount()
        XCTAssertEqual(activationRequestCount, 1)

        async let firstRefresh: Void = store.refreshLibrary()
        async let secondRefresh: Void = store.refreshLibrary(reason: .manualRetry)
        _ = await (firstRefresh, secondRefresh)
        let refreshRequestCount = await recorder.libraryRequestCount()
        XCTAssertEqual(refreshRequestCount, 2)
    }

    @MainActor
    func testXboxDataStoreInvalidatesOldGenerationAndReopensLibraryActivation() async throws {
        let recorder = XboxDataClientRecorder()
        let store = XboxDataStore(
            client: MockXboxDataClient(
                games: [Self.game],
                recorder: recorder,
                libraryDelay: .milliseconds(100),
                ignoresLibraryCancellation: true
            )
        )

        await store.sync(session: Self.storedSession, ownerGeneration: 7)
        let staleActivation = Task { @MainActor in
            await store.activateLibraryOnce()
        }
        try await waitUntilAsync {
            await recorder.libraryRequestCount() == 1
        }

        await store.sync(session: Self.storedSession, ownerGeneration: 8)
        await staleActivation.value

        XCTAssertEqual(store.libraryPhase, .idle)
        XCTAssertTrue(store.games.isEmpty)

        await store.activateLibraryOnce()
        await store.activateLibraryOnce()

        let requestCount = await recorder.libraryRequestCount()
        XCTAssertEqual(requestCount, 2)
        XCTAssertEqual(store.libraryPhase, .loaded)
        XCTAssertEqual(store.games, [Self.game])
    }

    @MainActor
    func testXboxDataStoreLoadsHostsWithRemotePlayIdentity() async {
        let host = XboxHostSummary(
            id: "stream-target",
            commandID: "console-command",
            streamTargetID: "stream-target",
            name: "客厅 Xbox",
            consoleType: "Series X",
            locale: "zh-CN",
            region: "CN",
            powerState: "ConnectedStandby",
            remoteManagementEnabled: true,
            consoleStreamingEnabled: true,
            wirelessWarning: false,
            outOfHomeWarning: false,
            storageDevices: []
        )
        let store = XboxDataStore(client: MockXboxDataClient(hosts: [host]))

        await store.sync(session: Self.storedSession)
        await store.activateHostsOnce()

        XCTAssertEqual(store.hostPhase, .loaded)
        XCTAssertEqual(store.hosts, [host])
        XCTAssertTrue(store.hosts[0].canStartRemotePlay)
    }

    @MainActor
    func testXboxDataStoreRunsPowerCommandSingleFlightAndRefreshesHosts() async throws {
        let host = XboxHostSummary(
            id: "stream-target",
            commandID: "console-command",
            streamTargetID: "stream-target",
            name: "客厅 Xbox",
            consoleType: "Series X",
            locale: "zh-CN",
            region: "CN",
            powerState: "ConnectedStandby",
            remoteManagementEnabled: true,
            consoleStreamingEnabled: true,
            wirelessWarning: false,
            outOfHomeWarning: false,
            storageDevices: []
        )
        let recorder = XboxDataClientRecorder()
        let store = XboxDataStore(
            client: MockXboxDataClient(
                hosts: [host],
                recorder: recorder,
                powerDelay: .milliseconds(100)
            )
        )
        await store.sync(session: Self.storedSession)
        await store.activateHostsOnce()

        let powerTask = Task { @MainActor in
            await store.powerOn(host: host)
        }
        try await waitUntil {
            store.hostPowerCommandState == .executing(
                hostID: host.id,
                command: .powerOn
            )
        }
        await store.powerOff(host: host)
        await powerTask.value

        let commands = await recorder.powerCommands()
        let hostRequestCount = await recorder.hostRequestCount()
        XCTAssertEqual(commands, [.powerOn(consoleID: "console-command")])
        XCTAssertEqual(hostRequestCount, 2)
        XCTAssertEqual(store.hostPowerCommandState, .idle)
    }

    @MainActor
    func testXboxDataStoreExposesRejectedPowerCommandError() async {
        let host = XboxHostSummary(
            id: "stream-target",
            commandID: "console-command",
            streamTargetID: "stream-target",
            name: "客厅 Xbox",
            consoleType: "Series X",
            locale: nil,
            region: nil,
            powerState: "On",
            remoteManagementEnabled: true,
            consoleStreamingEnabled: true,
            wirelessWarning: nil,
            outOfHomeWarning: nil,
            storageDevices: []
        )
        let recorder = XboxDataClientRecorder()
        let store = XboxDataStore(
            client: MockXboxDataClient(
                hosts: [host],
                recorder: recorder,
                powerAccepted: false
            )
        )
        await store.sync(session: Self.storedSession)
        await store.activateHostsOnce()

        await store.powerOff(host: host)

        XCTAssertEqual(
            store.hostPowerCommandState,
            .failed(
                hostID: host.id,
                command: .powerOff,
                message: "无法关闭主机，请稍后重试"
            )
        )
        XCTAssertEqual(store.hostPowerCommandState.errorMessage, "无法关闭主机，请稍后重试")
        let hostRequestCount = await recorder.hostRequestCount()
        XCTAssertEqual(hostRequestCount, 1)
    }

    @MainActor
    func testXboxDataStoreClearsContentAfterSignOut() async {
        let store = XboxDataStore(client: MockXboxDataClient(games: [Self.game]))
        await store.sync(session: Self.storedSession)
        await store.activateLibraryOnce()

        await store.sync(session: nil)

        XCTAssertEqual(store.libraryPhase, .idle)
        XCTAssertTrue(store.games.isEmpty)
        XCTAssertTrue(store.achievements(for: Self.game.titleID).isEmpty)
    }

    @MainActor
    func testXboxDataStoreKeepsLibraryWhenPlaytimeFails() async {
        let store = XboxDataStore(
            client: MockXboxDataClient(games: [Self.game], failsPlaytime: true)
        )

        await store.sync(session: Self.storedSession)
        await store.activateLibraryOnce()

        XCTAssertEqual(store.libraryPhase, .loaded)
        XCTAssertEqual(store.games, [Self.game])
        XCTAssertEqual(store.libraryErrorMessage, "游戏已载入，游玩时长暂时无法更新")
    }

    @MainActor
    func testXboxDataStoreCachesAchievementsUntilForcedRefresh() async {
        let recorder = XboxDataClientRecorder()
        let client = MockXboxDataClient(
            games: [Self.game],
            achievements: [Self.achievement],
            recorder: recorder
        )
        let store = XboxDataStore(client: client)
        await store.sync(session: Self.storedSession)

        await store.activateAchievementsOnce(for: Self.game)
        await store.activateAchievementsOnce(for: Self.game)
        let cachedRequestCount = await recorder.achievementRequestCount()
        XCTAssertEqual(cachedRequestCount, 1)
        XCTAssertEqual(store.achievements(for: Self.game.titleID), [Self.achievement])

        await store.refreshAchievements(for: Self.game)
        let refreshedRequestCount = await recorder.achievementRequestCount()
        XCTAssertEqual(refreshedRequestCount, 2)
    }

    @MainActor
    func testXboxDataStorePassesPreferredGameLocaleToAchievements() async {
        let recorder = XboxDataClientRecorder()
        let settings = MockPreferredGameLocaleProvider(preferredGameLocale: "zh-CN")
        let store = XboxDataStore(
            client: MockXboxDataClient(
                achievements: [Self.achievement],
                recorder: recorder
            ),
            preferredGameLocaleProvider: settings
        )
        await store.sync(session: Self.storedSession)

        await store.activateAchievementsOnce(for: Self.game)

        XCTAssertEqual(store.achievements(for: Self.game.titleID), [Self.achievement])
        let locales = await recorder.achievementLocales()
        XCTAssertEqual(locales, ["zh-CN"])
    }

    private static let profile = XboxProfile(
        xuid: "123",
        gamertag: "Player",
        displayName: "Player One",
        gamerScore: "1000",
        displayPictureUrl: "https://example.invalid/avatar.png",
        presenceState: "Online",
        presenceDevice: "Xbox Series X",
        currentTitleName: "Halo Infinite",
        richPresence: "多人游戏大厅",
        followersCount: 24,
        followingCount: 12,
        friendCount: 48
    )

    private static let storedSession = StoredAuthSession(
        refreshToken: "refresh",
        seedJSON: "seed",
        webTokenJSON: "web-token",
        appLevel: 0
    )

    private static let game = GameSummary(
        id: "1292135258",
        titleID: "1292135258",
        name: "Halo Infinite",
        artworkURL: URL(string: "https://example.invalid/box.png"),
        playtimeMinutes: nil,
        achievementProgress: AchievementProgress(
            unlockedCount: 10,
            totalCount: 50,
            earnedGamerscore: 200,
            totalGamerscore: 1000,
            percentage: 20
        )
    )

    private static let achievement = AchievementSummary(
        id: "1",
        titleID: "1292135258",
        name: "First Steps",
        description: "Complete the tutorial",
        imageURL: URL(string: "https://example.invalid/icon.png"),
        isSecret: false,
        isUnlocked: true,
        gamerscore: 25,
        progressPercentage: 100,
        unlockedAt: nil
    )

    private func makeGame(
        id: String,
        name: String,
        lastPlayedAt: Date? = nil,
        playtimeMinutes: Int? = nil,
        percentage: Int? = nil,
        earnedGamerscore: Int = 0
    ) -> GameSummary {
        GameSummary(
            id: id,
            titleID: id,
            name: name,
            artworkURL: nil,
            lastPlayedAt: lastPlayedAt,
            playtimeMinutes: playtimeMinutes,
            achievementProgress: percentage.map { percentage in
                AchievementProgress(
                    unlockedCount: 0,
                    totalCount: 0,
                    earnedGamerscore: earnedGamerscore,
                    totalGamerscore: 1_000,
                    percentage: percentage
                )
            }
        )
    }

    private func makeCloudGame(
        id: String,
        heroURL: URL? = nil,
        posterURL: URL? = nil,
        tileURL: URL? = nil,
        artworkURL: URL? = nil,
        isRecentlyPlayed: Bool = false,
        isNew: Bool = false,
        lastPlayedAt: Date? = nil
    ) -> CloudLibraryGame {
        CloudLibraryGame(
            productID: id,
            streamTitleID: "stream-\(id)",
            xboxTitleID: id,
            name: "Game \(id)",
            publisherName: "Xbox Game Studios",
            description: "Description",
            tileURL: tileURL,
            posterURL: posterURL,
            heroURL: heroURL,
            artworkURL: artworkURL,
            categories: ["Action"],
            supportedInputTypes: ["Controller"],
            hasEntitlement: true,
            isRecentlyPlayed: isRecentlyPlayed,
            isNew: isNew,
            lastPlayedAt: lastPlayedAt
                ?? (isRecentlyPlayed ? Date(timeIntervalSince1970: 1_000) : nil),
            playtimeMinutes: nil,
            achievementProgress: nil
        )
    }

    @MainActor
    func testStreamingFeatureStoreReachesPlayingThroughControlAndPeerStateMachine() async throws {
        let recorder = StreamingRuntimeRecorder()
        let control = StreamingTestControlSession(recorder: recorder)
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: control,
                recorder: recorder
            ),
            peerFactory: StreamingTestPeerFactory(recorder: recorder)
        )

        store.start(streamTitleID: "  1234abcd  ") {
            Self.streamingAccess(handle: "access-1")
        }

        try await waitUntil { store.videoTrack?.traceContext != nil }
        let currentContext = try XCTUnwrap(store.videoTrack?.traceContext)
        store.videoSurfaceRendererReady(
            context: StreamingPresentationTraceContext(
                attemptID: currentContext.attemptID,
                generation: currentContext.generation,
                peerEpoch: currentContext.peerEpoch &+ 1
            )
        )
        try await Task.sleep(for: .milliseconds(20))
        XCTAssertEqual(store.state, .waitingForFirstFrame)
        store.videoSurfaceRendererReady(context: currentContext)
        try await waitUntil { store.state == .playing }
        try await waitUntilAsync {
            (await recorder.snapshot()).remoteCandidatesApplied.count == 1
        }
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.streamTitleIDs, ["1234abcd"])
        XCTAssertEqual(snapshot.offers, ["local-offer"])
        XCTAssertEqual(snapshot.answersApplied, ["remote-answer"])
        XCTAssertEqual(snapshot.localCandidates, [
            StreamingIceCandidate(
                sdp: "candidate:local",
                sdpMid: "0",
                sdpMLineIndex: 0
            ),
        ])
        XCTAssertEqual(snapshot.remoteCandidatesApplied, [
            StreamingIceCandidate(
                sdp: "candidate:remote",
                sdpMid: "0",
                sdpMLineIndex: 0
            ),
        ])
        XCTAssertEqual(snapshot.markConnectedCalls, 1)
        XCTAssertEqual(snapshot.localIceCompletions, 1)
        XCTAssertNotNil(store.videoTrack)
        let traceContext = try XCTUnwrap(store.videoTrack?.traceContext)
        XCTAssertFalse(traceContext.attemptID.isEmpty)
        XCTAssertGreaterThan(traceContext.generation, 0)
        XCTAssertGreaterThan(traceContext.peerEpoch, 0)
    }

    @MainActor
    func testStreamingFeatureStoreWaitsForControlReadyAfterFirstFrame() async throws {
        let recorder = StreamingRuntimeRecorder()
        let peerFactory = StreamingTestPeerFactory(
            recorder: recorder,
            emitsControlReadyAutomatically: false,
            emitsFirstVideoFrameAutomatically: false
        )
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder),
                recorder: recorder
            ),
            peerFactory: peerFactory
        )

        store.start(streamTitleID: "1234abcd") {
            Self.streamingAccess(handle: "access-control-ready")
        }

        try await waitUntilAsync {
            (await recorder.snapshot()).markConnectedCalls == 1
        }
        peerFactory.emitFirstVideoFrame()
        try await markCurrentVideoSurfaceReady(store)
        try await Task.sleep(for: .milliseconds(20))
        XCTAssertEqual(store.state, .waitingForFirstFrame)

        peerFactory.emitControlReady()
        try await waitUntil { store.state == .playing }
    }

    @MainActor
    func testStreamingRemoteAnswerApplyTimeoutTerminatesNegotiation() async throws {
        let recorder = StreamingRuntimeRecorder()
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder),
                recorder: recorder
            ),
            peerFactory: StreamingTestPeerFactory(
                recorder: recorder,
                suspendAnswerApply: true
            ),
            remoteAnswerApplyTimeout: .milliseconds(40)
        )

        store.start(streamTitleID: "1234abcd") {
            Self.streamingAccess(handle: "access-answer-timeout")
        }

        try await waitUntil {
            if case let .failed(message, retryable) = store.state {
                return retryable
                    && message == StreamingRuntimeError.remoteDescriptionTimedOut.localizedDescription
            }
            return false
        }
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.answersApplied, ["remote-answer"])
        XCTAssertEqual(snapshot.peerStops, 1)
        XCTAssertEqual(snapshot.controlCloses, 1)
        XCTAssertEqual(snapshot.releasedAccessHandles, ["access-answer-timeout"])
    }

    @MainActor
    func testStreamingFeatureStorePrepareAccessFailureDoesNotCreateSession() async throws {
        let recorder = StreamingRuntimeRecorder()
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder),
                recorder: recorder
            ),
            peerFactory: StreamingTestPeerFactory(recorder: recorder)
        )

        store.start(streamTitleID: "1234abcd") {
            throw NSError(
                domain: "stream-access-fixture",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "access unavailable"]
            )
        }

        try await waitUntil {
            if case let .failed(_, retryable) = store.state {
                return retryable
            }
            return false
        }
        let snapshot = await recorder.snapshot()
        XCTAssertTrue(snapshot.targets.isEmpty)
        XCTAssertTrue(snapshot.releasedAccessHandles.isEmpty)
        XCTAssertEqual(snapshot.peerStops, 0)
        XCTAssertEqual(snapshot.controlCloses, 0)
    }

    func testStreamingOfferSdpProjectorConsumesDesktopWebRtcPlan() {
        let input = [
            "v=0",
            "m=audio 9 UDP/TLS/RTP/SAVPF 111",
            "a=rtpmap:111 opus/48000/2",
            "a=fmtp:111 minptime=10;useinbandfec=1",
            "m=video 9 UDP/TLS/RTP/SAVPF 96 97 98",
            "a=rtpmap:96 H264/90000",
            "a=fmtp:96 profile-level-id=640032;packetization-mode=0",
            "a=rtpmap:97 H264/90000",
            "a=fmtp:97 profile-level-id=4d0032;packetization-mode=0",
            "a=rtpmap:98 VP8/90000",
            "a=rtpmap:99 rtx/90000",
            "a=fmtp:99 apt=97;rtx-time=200",
            "",
        ].joined(separator: "\r\n")

        let output = StreamingOfferSdpProjector().project(
            input,
            plan: StreamingWebRtcPlan(
                h264Profiles: ["4d", "64"],
                maxFrameSize: 3_600,
                maxFrameRate: 60,
                minVideoBitrateKbps: 4_000,
                startVideoBitrateKbps: 8_000,
                maxVideoBitrateKbps: 12_000
            )
        )

        XCTAssertTrue(output.contains("m=video 9 UDP/TLS/RTP/SAVPF 97 96 98 99"))
        XCTAssertTrue(output.contains("b=AS:128"))
        XCTAssertTrue(output.contains("b=AS:12000"))
        XCTAssertTrue(output.contains("a=fmtp:97 profile-level-id=4d0032;packetization-mode=1;level-asymmetry-allowed=1;max-fs=3600;max-fr=60;x-google-min-bitrate=4000;x-google-start-bitrate=8000;x-google-max-bitrate=12000"))
        XCTAssertTrue(output.contains("a=fmtp:111 minptime=10;useinbandfec=1;stereo=1"))
        XCTAssertTrue(output.contains("a=rtcp-fb:97 transport-cc"))
        XCTAssertTrue(output.contains("a=rtcp-fb:97 nack pli"))
        XCTAssertTrue(output.contains("a=fmtp:99 apt=97;rtx-time=200"))
    }

    func testStreamingPreparedSignalingDefaultsToDesktopDirections() {
        let signaling = StreamingPreparedSignaling(iceServers: [])

        XCTAssertEqual(signaling.webRtcPlan.audioDirection, .sendReceive)
        XCTAssertEqual(signaling.webRtcPlan.videoDirection, .receiveOnly)
        XCTAssertEqual(signaling.webRtcPlan.h264PacketizationMode, 1)
        XCTAssertEqual(signaling.webRtcPlan.h264Profiles, ["4d", "42e", "420"])
    }

    func testStreamingIceCandidatePolicyFiltersAndOrdersDeterministically() {
        let candidates = [
            StreamingIceCandidate(
                sdp: "candidate:1 1 TCP 100 192.0.2.10 5000 typ host tcptype passive",
                sdpMid: "0",
                sdpMLineIndex: 0
            ),
            StreamingIceCandidate(
                sdp: "candidate:2 1 UDP 200 192.0.2.11 5001 typ srflx",
                sdpMid: "0",
                sdpMLineIndex: 0
            ),
            StreamingIceCandidate(
                sdp: "candidate:3 1 UDP 300 2001:db8::3 5002 typ host",
                sdpMid: "0",
                sdpMLineIndex: 0
            ),
            StreamingIceCandidate(
                sdp: "candidate:4 1 UDP 250 192.0.2.13 5002 typ host",
                sdpMid: "0",
                sdpMLineIndex: 0
            ),
            StreamingIceCandidate(
                sdp: "candidate:5 1 UDP 400 192.0.2.12 5003 typ relay",
                sdpMid: "0",
                sdpMLineIndex: 0
            ),
        ]
        let plan = StreamingWebRtcPlan(
            allowedCandidateTypes: ["host", "srflx"],
            preferIPv6: true
        )

        XCTAssertFalse(StreamingIceCandidatePolicy.allows(candidates[4], plan: plan))
        XCTAssertEqual(
            StreamingIceCandidatePolicy.ordered(candidates, plan: plan).map(\.sdp),
            [candidates[2].sdp, candidates[3].sdp, candidates[0].sdp, candidates[1].sdp]
        )
    }

    @MainActor
    func testStreamingDisconnectedPerformsIceRestartWithinSameSession() async throws {
        let recorder = StreamingRuntimeRecorder()
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder),
                recorder: recorder
            ),
            peerFactory: StreamingTestPeerFactory(
                recorder: recorder,
                transientDisconnectAfterConnect: true
            )
        )

        store.start(streamTitleID: "1234abcd") {
            Self.streamingAccess(handle: "access-restart")
        }

        try await waitUntilAsync {
            let snapshot = await recorder.snapshot()
            return snapshot.iceRestartOffers == [false, true]
                && snapshot.markConnectedCalls == 2
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }
        XCTAssertEqual(store.state, .playing)
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.peerStops, 0)
        XCTAssertEqual(snapshot.controlCloses, 0)
        XCTAssertTrue(snapshot.releasedAccessHandles.isEmpty)
    }

    @MainActor
    func testStreamingFailedIceRestartRebuildsPeerWithinSameRemoteSession() async throws {
        let recorder = StreamingRuntimeRecorder()
        let peerFactory = StreamingTestPeerFactory(
            recorder: recorder,
            transientDisconnectAfterConnect: true,
            failIceRestart: true
        )
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder),
                recorder: recorder
            ),
            peerFactory: peerFactory
        )

        store.start(streamTitleID: "1234abcd") {
            Self.streamingAccess(handle: "access-rebuild")
        }

        try await waitUntilAsync {
            let snapshot = await recorder.snapshot()
            return peerFactory.runtimesCreated == 2
                && snapshot.iceRestartOffers == [false, true, false]
                && snapshot.markConnectedCalls == 2
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }
        XCTAssertEqual(store.state, .playing)
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.peerStops, 1)
        XCTAssertEqual(snapshot.controlCloses, 0)
        XCTAssertTrue(snapshot.releasedAccessHandles.isEmpty)
    }

    @MainActor
    func testStreamingStopTakesOwnershipOfPeerRetiringDuringRuntimeRebuild() async throws {
        let recorder = StreamingRuntimeRecorder()
        let peerFactory = StreamingTestPeerFactory(
            recorder: recorder,
            transientDisconnectAfterConnect: true,
            failIceRestart: true,
            suspendFirstPeerStop: true
        )
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder),
                recorder: recorder
            ),
            peerFactory: peerFactory
        )

        store.start(streamTitleID: "1234abcd") {
            Self.streamingAccess(handle: "access-retiring-stop")
        }
        try await waitUntilAsync {
            (await recorder.snapshot()).peerInputStopStarts == 1
        }

        store.stop()
        try await Task.sleep(for: .milliseconds(20))
        await peerFactory.resumeFirstPeerStop()
        try await waitUntil { store.state == .idle }

        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.peerInputStopStarts, 1)
        XCTAssertEqual(snapshot.peerStops, 1)
        XCTAssertEqual(snapshot.controlCloses, 1)
        XCTAssertEqual(snapshot.releasedAccessHandles, ["access-retiring-stop"])
        XCTAssertEqual(snapshot.cleanupEvents, [
            "inputStopped", "hapticsStopped", "peerClosed", "remoteSessionClosed", "accessReleased",
        ])
    }

    @MainActor
    func testStreamingIceRestartWaitsForPreviousEpochCandidateMutation() async throws {
        let recorder = StreamingRuntimeRecorder()
        let submitGate = StreamingAsyncGate()
        let control = StreamingTestControlSession(
            recorder: recorder,
            firstLocalCandidateSubmitGate: submitGate
        )
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(session: control, recorder: recorder),
            peerFactory: StreamingTestPeerFactory(
                recorder: recorder,
                transientDisconnectAfterConnect: true
            )
        )

        store.start(streamTitleID: "1234abcd") {
            Self.streamingAccess(handle: "access-ice-epoch")
        }
        try await waitUntilAsync {
            (await recorder.snapshot()).localCandidateSubmitStarts == 1
        }
        try await Task.sleep(for: .milliseconds(40))

        var snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.iceRestartOffers, [false])
        XCTAssertEqual(snapshot.localCandidateSubmitFinishes, 0)
        XCTAssertEqual(snapshot.localIceCompletions, 0)

        await submitGate.open()
        try await waitUntilAsync {
            let current = await recorder.snapshot()
            return current.iceRestartOffers == [false, true]
                && current.localCandidateSubmitFinishes == 2
                && current.localIceCompletions == 1
                && current.markConnectedCalls == 2
        }
        snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.localCandidateSubmitStarts, 2)
        XCTAssertEqual(snapshot.localCandidateSubmitFinishes, 2)
        XCTAssertEqual(snapshot.localIceCompletions, 1)
    }

    @MainActor
    func testStreamingFailedAndClosedUseIndependentTerminalRetryability() async throws {
        for (terminal, expectedRetryable) in [
            (StreamingPeerConnectionState.failed, true),
            (.closed, false),
        ] {
            let recorder = StreamingRuntimeRecorder()
            let store = StreamingFeatureStore(
                controlFactory: StreamingTestControlFactory(
                    session: StreamingTestControlSession(recorder: recorder),
                    recorder: recorder
                ),
                peerFactory: StreamingTestPeerFactory(
                    recorder: recorder,
                    terminalStateAfterConnect: terminal
                )
            )
            store.start(streamTitleID: "1234abcd") {
                Self.streamingAccess(handle: "access-\(terminal.rawValue)")
            }

            try await waitUntil {
                if case let .failed(_, retryable) = store.state {
                    return retryable == expectedRetryable
                }
                return false
            }
            let snapshot = await recorder.snapshot()
            XCTAssertEqual(snapshot.peerStops, 1)
            XCTAssertEqual(snapshot.controlCloses, 1)
            XCTAssertEqual(snapshot.releasedAccessHandles, ["access-\(terminal.rawValue)"])
        }
    }

    @MainActor
    func testStreamingFeatureStoreHomeLaunchUsesSelectedConsoleTarget() async throws {
        let recorder = StreamingRuntimeRecorder()
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder),
                recorder: recorder
            ),
            peerFactory: StreamingTestPeerFactory(recorder: recorder)
        )

        store.startHome(targetID: "  console-server-id  ") {
            Self.streamingHomeAccess(handle: "home-access-1")
        }

        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }
        try await waitUntilAsync {
            (await recorder.snapshot()).remoteCandidatesApplied.count == 1
        }
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.targets, [.home])
        XCTAssertEqual(snapshot.streamTitleIDs, ["console-server-id"])
        XCTAssertEqual(snapshot.remoteCandidatesApplied.map(\.sdp), ["candidate:remote"])
        XCTAssertEqual(store.streamTarget, .home)
        XCTAssertEqual(store.streamTitleID, "console-server-id")
    }

    @MainActor
    func testStreamingFeatureStoreForwardsConsumedSettingsIntoLaunchRequest() async throws {
        let suiteName = "XBXRC.StreamingLaunchSettings.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suiteName)!
        defer { defaults.removePersistentDomain(forName: suiteName) }

        let settingsStore = AppSettingsStore(defaults: defaults)
        settingsStore.preferredGameLocale = "ko-KR"
        settingsStore.cloudResolution = .p1440
        settingsStore.homeResolution = .p720
        settingsStore.preferIPv6 = true
        settingsStore.codecPreference = .h264High
        settingsStore.homeBitrateMode = .custom
        settingsStore.homeBitrateMbps = 28
        settingsStore.cloudBitrateMode = .custom
        settingsStore.cloudBitrateMbps = 16
        settingsStore.audioBitrateMode = .custom
        settingsStore.audioBitrateKbps = 160
        settingsStore.homeTurnFallbackEnabled = false

        let recorder = StreamingRuntimeRecorder()
        let store = StreamingFeatureStore(
            settingsProvider: settingsStore,
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder),
                recorder: recorder
            ),
            peerFactory: StreamingTestPeerFactory(recorder: recorder)
        )

        store.start(streamTitleID: "1234abcd") {
            Self.streamingAccess(handle: "access-settings")
        }

        try await waitUntilAsync {
            !(await recorder.snapshot()).settings.isEmpty
        }
        let snapshot = await recorder.snapshot()
        let settings = try XCTUnwrap(snapshot.settings.first)
        XCTAssertEqual(settings.preferredGameLocale, "ko-KR")
        XCTAssertEqual(settings.cloudResolution, 1440)
        XCTAssertEqual(settings.homeResolution, 720)
        XCTAssertTrue(settings.preferIPv6)
        XCTAssertEqual(settings.videoCodec, "video/H264-64")
        XCTAssertEqual(settings.homeBitrateMode, "Custom")
        XCTAssertEqual(settings.homeBitrateMbps, 28)
        XCTAssertEqual(settings.cloudBitrateMode, "Custom")
        XCTAssertEqual(settings.cloudBitrateMbps, 16)
        XCTAssertEqual(settings.audioBitrateMode, "Custom")
        XCTAssertEqual(settings.audioBitrateKbps, 160)
        XCTAssertFalse(settings.homeTurnFallback)
    }

    @MainActor
    func testStreamingFeatureStoreStopIsIdempotentAndReleasesAllOwners() async throws {
        let recorder = StreamingRuntimeRecorder()
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder),
                recorder: recorder
            ),
            peerFactory: StreamingTestPeerFactory(recorder: recorder)
        )
        store.start(streamTitleID: "1234abcd") {
            Self.streamingAccess(handle: "access-stop")
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }

        store.stop()
        store.stop()
        try await waitUntil { store.state == .idle }

        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.peerStops, 1)
        XCTAssertEqual(snapshot.controlCloses, 1)
        XCTAssertEqual(snapshot.releasedAccessHandles, ["access-stop"])
        XCTAssertEqual(snapshot.cleanupEvents, [
            "inputStopped", "hapticsStopped", "peerClosed", "remoteSessionClosed", "accessReleased",
        ])
        XCTAssertNil(store.videoTrack)
    }

    @MainActor
    func testStreamingFeatureStoreBackgroundPhaseClosesActiveSession() async throws {
        let recorder = StreamingRuntimeRecorder()
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder),
                recorder: recorder
            ),
            peerFactory: StreamingTestPeerFactory(recorder: recorder)
        )
        store.start(streamTitleID: "1234abcd") {
            Self.streamingAccess(handle: "access-background")
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }

        store.handleScenePhase(.background)
        try await waitUntil { store.state == .idle }

        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.peerStops, 1)
        XCTAssertEqual(snapshot.controlCloses, 1)
        XCTAssertEqual(snapshot.releasedAccessHandles, ["access-background"])
    }

    @MainActor
    func testStreamingFeatureStoreStaleStopDoesNotCloseRestartedSession() async throws {
        let recorder = StreamingRuntimeRecorder()
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder),
                recorder: recorder
            ),
            peerFactory: StreamingTestPeerFactory(recorder: recorder)
        )
        store.start(streamTitleID: "1111aaaa") {
            Self.streamingAccess(handle: "access-first")
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }

        store.stop()
        store.start(streamTitleID: "2222bbbb") {
            Self.streamingAccess(handle: "access-second")
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing && store.streamTitleID == "2222bbbb" }
        try await Task.sleep(for: .milliseconds(100))

        XCTAssertEqual(store.state, .playing)
        XCTAssertEqual(store.streamTitleID, "2222bbbb")
        store.stop()
        try await waitUntil { store.state == .idle }

        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.streamTitleIDs, ["1111aaaa", "2222bbbb"])
        XCTAssertEqual(snapshot.peerStops, 2)
        XCTAssertEqual(snapshot.controlCloses, 2)
        XCTAssertEqual(snapshot.releasedAccessHandles, ["access-first", "access-second"])
    }

    @MainActor
    private func markCurrentVideoSurfaceReady(_ store: StreamingFeatureStore) async throws {
        try await waitUntil { store.videoTrack?.traceContext != nil }
        store.videoSurfaceRendererReady(
            context: try XCTUnwrap(store.videoTrack?.traceContext)
        )
    }

    @MainActor
    private func waitUntil(
        timeout: Duration = .seconds(2),
        condition: @escaping @MainActor () -> Bool
    ) async throws {
        let clock = ContinuousClock()
        let deadline = clock.now.advanced(by: timeout)
        while !condition() {
            guard clock.now < deadline else {
                XCTFail("等待串流状态超时")
                return
            }
            try await Task.sleep(for: .milliseconds(10))
        }
    }

    @MainActor
    private func waitUntilAsync(
        timeout: Duration = .seconds(2),
        condition: @escaping @MainActor () async -> Bool
    ) async throws {
        let clock = ContinuousClock()
        let deadline = clock.now.advanced(by: timeout)
        while !(await condition()) {
            guard clock.now < deadline else {
                XCTFail("等待异步串流状态超时")
                return
            }
            try await Task.sleep(for: .milliseconds(10))
        }
    }

    private static func streamingAccess(handle: String) -> PreparedCloudAccess {
        PreparedCloudAccess(
            authSession: AuthSession(
                refreshToken: "refresh",
                seedJson: "seed",
                webTokenJson: "web-token",
                appLevel: 2
            ),
            handle: handle,
            accountID: "account",
            regionHost: "region.example"
        )
    }

    private static func streamingHomeAccess(handle: String) -> PreparedHomeAccess {
        PreparedHomeAccess(
            authSession: AuthSession(
                refreshToken: "refresh",
                seedJson: "seed",
                webTokenJson: "web-token",
                appLevel: 1
            ),
            handle: handle,
            accountID: "account",
            regionHost: "home.example"
        )
    }

    private func traceEnvelopes(
        from writer: IOSRuntimeTraceWriter
    ) async throws -> [IOSRuntimeTraceEnvelope] {
        let decoder = JSONDecoder()
        let files = await writer.traceFiles()
        return try files.flatMap { file in
            try String(contentsOf: file, encoding: .utf8)
                .split(separator: "\n")
                .map { line in
                    try decoder.decode(
                        IOSRuntimeTraceEnvelope.self,
                        from: Data(line.utf8)
                    )
                }
        }
    }
}

private actor StreamingAsyncGate {
    private var isOpen = false
    private var waiters: [CheckedContinuation<Void, Never>] = []

    func wait() async {
        if isOpen { return }
        await withCheckedContinuation { continuation in
            waiters.append(continuation)
        }
    }

    func open() {
        isOpen = true
        let pending = waiters
        waiters.removeAll()
        for continuation in pending {
            continuation.resume()
        }
    }
}

private struct StreamingRuntimeRecorderSnapshot: Sendable {
    let targets: [StreamingLaunchTarget]
    let streamTitleIDs: [String]
    let settings: [StreamingSessionSettingsSnapshot]
    let offers: [String]
    let answersApplied: [String]
    let localCandidates: [StreamingIceCandidate]
    let remoteCandidatesApplied: [StreamingIceCandidate]
    let releasedAccessHandles: [String]
    let peerStops: Int
    let controlCloses: Int
    let markConnectedCalls: Int
    let localIceCompletions: Int
    let iceRestartOffers: [Bool]
    let cleanupEvents: [String]
    let peerInputStopStarts: Int
    let localCandidateSubmitStarts: Int
    let localCandidateSubmitFinishes: Int
}

private actor StreamingRuntimeRecorder {
    private var targets: [StreamingLaunchTarget] = []
    private var streamTitleIDs: [String] = []
    private var settings: [StreamingSessionSettingsSnapshot] = []
    private var offers: [String] = []
    private var answersApplied: [String] = []
    private var localCandidates: [StreamingIceCandidate] = []
    private var remoteCandidatesApplied: [StreamingIceCandidate] = []
    private var releasedAccessHandles: [String] = []
    private var peerStops = 0
    private var controlCloses = 0
    private var markConnectedCalls = 0
    private var localIceCompletions = 0
    private var iceRestartOffers: [Bool] = []
    private var cleanupEvents: [String] = []
    private var peerInputStopStarts = 0
    private var localCandidateSubmitStarts = 0
    private var localCandidateSubmitFinishes = 0

    func recordTarget(_ value: StreamingLaunchTarget) { targets.append(value) }
    func recordTitleID(_ value: String) { streamTitleIDs.append(value) }
    func recordSettings(_ value: StreamingSessionSettingsSnapshot) { settings.append(value) }
    func recordOffer(_ value: String) { offers.append(value) }
    func recordAnswer(_ value: String) { answersApplied.append(value) }
    func recordCandidate(_ value: StreamingIceCandidate) { localCandidates.append(value) }
    func recordRemoteCandidates(_ values: [StreamingIceCandidate]) {
        remoteCandidatesApplied.append(contentsOf: values)
    }
    func recordRelease(_ value: String) { releasedAccessHandles.append(value) }
    func recordPeerStop() { peerStops += 1 }
    func recordControlClose() { controlCloses += 1 }
    func recordMarkConnected() { markConnectedCalls += 1 }
    func recordLocalIceCompletion() { localIceCompletions += 1 }
    func recordIceRestartOffer(_ value: Bool) { iceRestartOffers.append(value) }
    func recordCleanup(_ event: String) { cleanupEvents.append(event) }
    func recordPeerInputStopStarted() { peerInputStopStarts += 1 }
    func recordLocalCandidateSubmitStarted() { localCandidateSubmitStarts += 1 }
    func recordLocalCandidateSubmitFinished() { localCandidateSubmitFinishes += 1 }

    func snapshot() -> StreamingRuntimeRecorderSnapshot {
        StreamingRuntimeRecorderSnapshot(
            targets: targets,
            streamTitleIDs: streamTitleIDs,
            settings: settings,
            offers: offers,
            answersApplied: answersApplied,
            localCandidates: localCandidates,
            remoteCandidatesApplied: remoteCandidatesApplied,
            releasedAccessHandles: releasedAccessHandles,
            peerStops: peerStops,
            controlCloses: controlCloses,
            markConnectedCalls: markConnectedCalls,
            localIceCompletions: localIceCompletions,
            iceRestartOffers: iceRestartOffers,
            cleanupEvents: cleanupEvents,
            peerInputStopStarts: peerInputStopStarts,
            localCandidateSubmitStarts: localCandidateSubmitStarts,
            localCandidateSubmitFinishes: localCandidateSubmitFinishes
        )
    }
}

private struct StreamingTestControlFactory: StreamingControlSessionFactory {
    let session: StreamingTestControlSession
    let recorder: StreamingRuntimeRecorder

    func createSession(request: StreamingLaunchRequest) async throws -> any StreamingControlSession {
        await recorder.recordTarget(request.target)
        await recorder.recordTitleID(request.streamTitleID)
        await recorder.recordSettings(request.settings)
        return session
    }

    func releaseAccess(handle: String) async {
        await recorder.recordRelease(handle)
        await recorder.recordCleanup("accessReleased")
    }
}

private actor StreamingTestControlSession: StreamingControlSession {
    let recorder: StreamingRuntimeRecorder
    private let firstLocalCandidateSubmitGate: StreamingAsyncGate?
    private var localCandidateSubmitCount = 0

    init(
        recorder: StreamingRuntimeRecorder,
        firstLocalCandidateSubmitGate: StreamingAsyncGate? = nil
    ) {
        self.recorder = recorder
        self.firstLocalCandidateSubmitGate = firstLocalCandidateSubmitGate
    }

    func prepareSignaling() async throws -> StreamingPreparedSignaling {
        return StreamingPreparedSignaling(
            iceServers: []
        )
    }

    func exchangeOffer(_ sdp: String) async throws -> String {
        await recorder.recordOffer(sdp)
        return "remote-answer"
    }

    func submitLocalCandidates(_ candidates: [StreamingIceCandidate]) async throws {
        localCandidateSubmitCount += 1
        await recorder.recordLocalCandidateSubmitStarted()
        if localCandidateSubmitCount == 1, let firstLocalCandidateSubmitGate {
            await firstLocalCandidateSubmitGate.wait()
        }
        for candidate in candidates {
            await recorder.recordCandidate(candidate)
        }
        await recorder.recordLocalCandidateSubmitFinished()
    }

    func completeLocalIceGathering() async throws {
        await recorder.recordLocalIceCompletion()
    }

    func nextRemoteIceBatch() async throws -> StreamingRemoteIceBatch {
        StreamingRemoteIceBatch(
            candidates: [
                StreamingIceCandidate(
                    sdp: "candidate:remote",
                    sdpMid: "0",
                    sdpMLineIndex: 0
                ),
            ],
            endOfCandidates: true
        )
    }

    func markConnected() async throws {
        await recorder.recordMarkConnected()
    }

    func close() async {
        await recorder.recordControlClose()
        await recorder.recordCleanup("remoteSessionClosed")
    }
}

private final class StreamingTestPeerFactory: StreamingPeerRuntimeFactory, @unchecked Sendable {
    let recorder: StreamingRuntimeRecorder
    let transientDisconnectAfterConnect: Bool
    let terminalStateAfterConnect: StreamingPeerConnectionState?
    let failIceRestart: Bool
    let suspendAnswerApply: Bool
    let emitsControlReadyAutomatically: Bool
    let emitsFirstVideoFrameAutomatically: Bool
    let firstPeerStopGate: StreamingAsyncGate?
    private(set) var runtimesCreated = 0
    private var latestEventSink: (@Sendable (StreamingPeerEvent) -> Void)?

    init(
        recorder: StreamingRuntimeRecorder,
        transientDisconnectAfterConnect: Bool = false,
        terminalStateAfterConnect: StreamingPeerConnectionState? = nil,
        failIceRestart: Bool = false,
        suspendAnswerApply: Bool = false,
        emitsControlReadyAutomatically: Bool = true,
        emitsFirstVideoFrameAutomatically: Bool = true,
        suspendFirstPeerStop: Bool = false
    ) {
        self.recorder = recorder
        self.transientDisconnectAfterConnect = transientDisconnectAfterConnect
        self.terminalStateAfterConnect = terminalStateAfterConnect
        self.failIceRestart = failIceRestart
        self.suspendAnswerApply = suspendAnswerApply
        self.emitsControlReadyAutomatically = emitsControlReadyAutomatically
        self.emitsFirstVideoFrameAutomatically = emitsFirstVideoFrameAutomatically
        firstPeerStopGate = suspendFirstPeerStop ? StreamingAsyncGate() : nil
    }

    func makeRuntime(
        eventSink: @escaping @Sendable (StreamingPeerEvent) -> Void
    ) -> any StreamingPeerRuntime {
        runtimesCreated += 1
        latestEventSink = eventSink
        let isFirstRuntime = runtimesCreated == 1
        return StreamingTestPeerRuntime(
            eventSink: eventSink,
            recorder: recorder,
            transientDisconnectAfterConnect: transientDisconnectAfterConnect && isFirstRuntime,
            terminalStateAfterConnect: isFirstRuntime ? terminalStateAfterConnect : nil,
            failIceRestart: failIceRestart && isFirstRuntime,
            suspendAnswerApply: suspendAnswerApply && isFirstRuntime,
            emitsControlReadyAutomatically: emitsControlReadyAutomatically,
            emitsFirstVideoFrameAutomatically: emitsFirstVideoFrameAutomatically,
            stopGate: isFirstRuntime ? firstPeerStopGate : nil
        )
    }

    func emitFirstVideoFrame() {
        latestEventSink?(.firstVideoFrame)
    }

    func emitControlReady() {
        latestEventSink?(.dataChannelEvent(label: "control", event: "ready"))
    }

    func resumeFirstPeerStop() async {
        await firstPeerStopGate?.open()
    }
}

private final class StreamingTestPeerRuntime: StreamingPeerRuntime, @unchecked Sendable {
    private let eventSink: @Sendable (StreamingPeerEvent) -> Void
    private let recorder: StreamingRuntimeRecorder
    private let transientDisconnectAfterConnect: Bool
    private let terminalStateAfterConnect: StreamingPeerConnectionState?
    private let failIceRestart: Bool
    private let suspendAnswerApply: Bool
    private let emitsControlReadyAutomatically: Bool
    private let emitsFirstVideoFrameAutomatically: Bool
    private let stopGate: StreamingAsyncGate?
    private var offerCount = 0

    init(
        eventSink: @escaping @Sendable (StreamingPeerEvent) -> Void,
        recorder: StreamingRuntimeRecorder,
        transientDisconnectAfterConnect: Bool,
        terminalStateAfterConnect: StreamingPeerConnectionState?,
        failIceRestart: Bool,
        suspendAnswerApply: Bool,
        emitsControlReadyAutomatically: Bool,
        emitsFirstVideoFrameAutomatically: Bool,
        stopGate: StreamingAsyncGate?
    ) {
        self.eventSink = eventSink
        self.recorder = recorder
        self.transientDisconnectAfterConnect = transientDisconnectAfterConnect
        self.terminalStateAfterConnect = terminalStateAfterConnect
        self.failIceRestart = failIceRestart
        self.suspendAnswerApply = suspendAnswerApply
        self.emitsControlReadyAutomatically = emitsControlReadyAutomatically
        self.emitsFirstVideoFrameAutomatically = emitsFirstVideoFrameAutomatically
        self.stopGate = stopGate
    }

    func makeOffer(
        configuration _: StreamingPreparedSignaling,
        iceRestart: Bool
    ) async throws -> String {
        offerCount += 1
        await recorder.recordIceRestartOffer(iceRestart)
        if iceRestart, failIceRestart {
            throw StreamingRuntimeError.peerConnectionFailed
        }
        eventSink(
            .localCandidate(
                StreamingIceCandidate(
                    sdp: "candidate:local",
                    sdpMid: "0",
                    sdpMLineIndex: 0
                )
            )
        )
        eventSink(.localIceGatheringComplete)
        return "local-offer"
    }

    func applyAnswer(_ sdp: String) async throws {
        await recorder.recordAnswer(sdp)
        if suspendAnswerApply {
            try await Task.sleep(for: .seconds(60))
        }
        if offerCount == 1 {
            eventSink(.dataChannelEvent(label: "all", event: "profilesCreated"))
            eventSink(.dataChannelEvent(label: "message", event: "handshakeAcked"))
            if emitsControlReadyAutomatically {
                eventSink(.dataChannelEvent(label: "control", event: "ready"))
            }
        }
        eventSink(.videoTrack(StreamingVideoTrackHandle(rawValue: NSObject())))
        eventSink(.audioTrackReady)
        eventSink(.connectionStateChanged(.connected))
        eventSink(.connectionStateChanged(.connected))
        if emitsFirstVideoFrameAutomatically {
            eventSink(.firstVideoFrame)
        }
        if offerCount == 1, transientDisconnectAfterConnect {
            Task { [eventSink] in
                try? await Task.sleep(for: .milliseconds(1))
                eventSink(.connectionStateChanged(.disconnected))
            }
        } else if offerCount == 1, let terminalStateAfterConnect {
            eventSink(.connectionStateChanged(terminalStateAfterConnect))
            eventSink(.connectionStateChanged(terminalStateAfterConnect))
        }
    }

    func addRemoteCandidates(_ candidates: [StreamingIceCandidate]) async throws {
        await recorder.recordRemoteCandidates(candidates)
    }

    func debugSnapshot() async -> StreamingPeerDebugSnapshot {
        StreamingPeerDebugSnapshot(
            signalingState: suspendAnswerApply ? "haveLocalOffer" : "stable",
            iceConnectionState: suspendAnswerApply ? "checking" : "connected",
            iceGatheringState: "complete",
            transceiverCount: 2,
            audioReceiverTrackCount: suspendAnswerApply ? 0 : 1,
            videoReceiverTrackCount: suspendAnswerApply ? 0 : 1,
            localDescriptionSet: true,
            remoteDescriptionSet: !suspendAnswerApply,
            dataChannels: StreamingDataChannelDebugSnapshot(
                readyStates: [
                    "input": suspendAnswerApply ? "connecting" : "open",
                    "control": suspendAnswerApply ? "connecting" : "open",
                    "chat": suspendAnswerApply ? "connecting" : "open",
                    "message": suspendAnswerApply ? "connecting" : "open",
                ],
                phases: [
                    "input": "created",
                    "control": "created",
                    "chat": "created",
                    "message": "created",
                ],
                handshakeAcknowledged: false,
                controlReady: false,
                inputStarted: false,
                terminalReason: nil
            )
        )
    }

    func stopInputAndHaptics() async {
        await recorder.recordPeerInputStopStarted()
        await stopGate?.wait()
        await recorder.recordCleanup("inputStopped")
        await recorder.recordCleanup("hapticsStopped")
    }

    func closeTransport() async {
        await recorder.recordPeerStop()
        await recorder.recordCleanup("peerClosed")
    }
}

private actor InMemoryCloudCatalogSnapshotStore: CloudCatalogSnapshotStoring {
    private var snapshots: [CloudCatalogScope: CloudCatalogSnapshot] = [:]

    init(snapshot: CloudCatalogSnapshot? = nil) {
        if let snapshot {
            snapshots[snapshot.scope] = snapshot
        }
    }

    func load(scope: CloudCatalogScope) async throws -> CloudCatalogSnapshot? {
        snapshots[scope]
    }

    func save(_ snapshot: CloudCatalogSnapshot) async throws {
        snapshots[snapshot.scope] = snapshot
    }

    func clearOverlay(accountID: String) async throws {
        snapshots = snapshots.filter { $0.key.accountID != accountID }
    }
}

private actor MockXboxCloudDataClient: XboxCloudDataClient {
    private let snapshot: RemoteCloudCatalogSnapshot
    private var catalogRequests = 0

    init(snapshot: RemoteCloudCatalogSnapshot) {
        self.snapshot = snapshot
    }

    func prepareAccess(
        refreshToken _: String,
        seedJSON _: String,
        forceRegionIP _: String
    ) async throws -> PreparedCloudAccess {
        PreparedCloudAccess(
            authSession: AuthSession(
                refreshToken: "refresh",
                seedJson: "seed",
                webTokenJson: "web",
                appLevel: 2
            ),
            handle: "handle",
            accountID: snapshot.scope.accountID,
            regionHost: snapshot.scope.regionHost
        )
    }

    func prepareHomeAccess(
        refreshToken _: String,
        seedJSON _: String,
        forceRegionIP _: String
    ) async throws -> PreparedHomeAccess {
        PreparedHomeAccess(
            authSession: AuthSession(
                refreshToken: "refresh",
                seedJson: "seed",
                webTokenJson: "web",
                appLevel: 1
            ),
            handle: "home-handle",
            accountID: snapshot.scope.accountID,
            regionHost: "home.example.com"
        )
    }

    func loadCatalog(
        accessHandle _: String,
        market _: String,
        language _: String
    ) async throws -> RemoteCloudCatalogSnapshot {
        catalogRequests += 1
        try await Task.sleep(for: .milliseconds(40))
        return snapshot
    }

    func loadMetadataPage(
        accessHandle _: String,
        market _: String,
        language _: String,
        productIDs _: [String]
    ) async throws -> [CloudCatalogMetadata] {
        []
    }

    func releaseAccess(handle _: String) async {}

    func catalogRequestCount() -> Int {
        catalogRequests
    }
}

private actor InMemoryAuthSessionStore: AuthSessionStoring {
    private var session: StoredAuthSession?

    init(session: StoredAuthSession? = nil) {
        self.session = session
    }

    func load() async throws -> StoredAuthSession? {
        session
    }

    func save(_ session: StoredAuthSession) async throws {
        self.session = session
    }

    func delete() async throws {
        session = nil
    }

    func currentSession() -> StoredAuthSession? {
        session
    }
}

private struct MockXboxAuthClient: XboxAuthClient {
    let renewedSession: AuthSession
    let finishedSession: AuthSession
    let profile: XboxProfile
    let recorder: RegionRoutingRecorder?

    init(
        renewedSession: AuthSession = AuthSession(
            refreshToken: "refresh",
            seedJson: "seed",
            webTokenJson: "web-token",
            appLevel: 1
        ),
        finishedSession: AuthSession = AuthSession(
            refreshToken: "refresh",
            seedJson: "seed",
            webTokenJson: "web-token",
            appLevel: 1
        ),
        profile: XboxProfile = XboxProfile(
            xuid: nil,
            gamertag: "Player",
            displayName: "Player",
            gamerScore: "0",
            displayPictureUrl: "",
            presenceState: nil,
            presenceDevice: nil,
            currentTitleName: nil,
            richPresence: nil,
            followersCount: nil,
            followingCount: nil,
            friendCount: nil
        ),
        recorder: RegionRoutingRecorder? = nil
    ) {
        self.renewedSession = renewedSession
        self.finishedSession = finishedSession
        self.profile = profile
        self.recorder = recorder
    }

    func beginLogin() async throws -> LoginStartResult {
        LoginStartResult(
            authorizationUrl: "https://login.live.com/oauth20_authorize.srf",
            state: "state",
            pendingJson: "pending",
            seedJson: "seed"
        )
    }

    func finishLogin(
        callbackURL _: URL,
        pendingJSON _: String,
        seedJSON _: String,
        forceRegionIP: String
    ) async throws -> AuthSession {
        await recorder?.recordFinish(forceRegionIP)
        return finishedSession
    }

    func renewLogin(
        refreshToken _: String,
        seedJSON _: String,
        forceRegionIP: String
    ) async throws -> AuthSession {
        await recorder?.recordRenew(forceRegionIP)
        return renewedSession
    }

    func loadProfile(webTokenJSON _: String) async throws -> XboxProfile {
        await recorder?.recordProfileRequest()
        profile
    }
}

private actor RegionRoutingRecorder {
    private var finishRegionIP: String?
    private var renewRegionIP: String?
    private var profileRequests = 0

    func recordFinish(_ value: String) {
        finishRegionIP = value
    }

    func recordRenew(_ value: String) {
        renewRegionIP = value
    }

    func recordProfileRequest() {
        profileRequests += 1
    }

    func lastRenewRegionIP() -> String? {
        renewRegionIP
    }

    func profileRequestCount() -> Int {
        profileRequests
    }
}

@MainActor
private final class MockCloudRegionSettings: CloudRegionSettingsProviding {
    let cloudRegionPreset: CloudRegionPreset
    let usesEphemeralLoginSession: Bool

    init(preset: CloudRegionPreset, usesEphemeralLoginSession: Bool = false) {
        cloudRegionPreset = preset
        self.usesEphemeralLoginSession = usesEphemeralLoginSession
    }
}

private struct MockXboxDataClient: XboxDataClient {
    let hosts: [XboxHostSummary]
    let games: [GameSummary]
    let playtimes: [TitlePlaytime]
    let achievements: [AchievementSummary]
    let recorder: XboxDataClientRecorder?
    let failsPlaytime: Bool
    let powerAccepted: Bool
    let powerDelay: Duration?
    let libraryDelay: Duration?
    let ignoresLibraryCancellation: Bool

    init(
        hosts: [XboxHostSummary] = [],
        games: [GameSummary] = [],
        playtimes: [TitlePlaytime] = [],
        achievements: [AchievementSummary] = [],
        recorder: XboxDataClientRecorder? = nil,
        failsPlaytime: Bool = false,
        powerAccepted: Bool = true,
        powerDelay: Duration? = nil,
        libraryDelay: Duration? = nil,
        ignoresLibraryCancellation: Bool = false
    ) {
        self.hosts = hosts
        self.games = games
        self.playtimes = playtimes
        self.achievements = achievements
        self.recorder = recorder
        self.failsPlaytime = failsPlaytime
        self.powerAccepted = powerAccepted
        self.powerDelay = powerDelay
        self.libraryDelay = libraryDelay
        self.ignoresLibraryCancellation = ignoresLibraryCancellation
    }

    func loadHosts(webTokenJSON _: String) async throws -> [XboxHostSummary] {
        await recorder?.recordHostRequest()
        return hosts
    }

    func powerOn(
        webTokenJSON _: String,
        consoleID: String
    ) async throws -> HostPowerCommandResult {
        await recorder?.recordPowerCommand(.powerOn(consoleID: consoleID))
        if let powerDelay {
            try await Task.sleep(for: powerDelay)
        }
        return HostPowerCommandResult(consoleID: consoleID, accepted: powerAccepted)
    }

    func powerOff(
        webTokenJSON _: String,
        consoleID: String
    ) async throws -> HostPowerCommandResult {
        await recorder?.recordPowerCommand(.powerOff(consoleID: consoleID))
        if let powerDelay {
            try await Task.sleep(for: powerDelay)
        }
        return HostPowerCommandResult(consoleID: consoleID, accepted: powerAccepted)
    }

    func loadGameLibrary(webTokenJSON _: String) async throws -> [GameSummary] {
        await recorder?.recordLibraryRequest()
        if let libraryDelay {
            if ignoresLibraryCancellation {
                try? await Task.sleep(for: libraryDelay)
            } else {
                try await Task.sleep(for: libraryDelay)
            }
        }
        games
    }

    func loadPlaytimes(
        webTokenJSON _: String,
        titleIDs _: [String]
    ) async throws -> [TitlePlaytime] {
        if failsPlaytime {
            throw MockXboxDataError.unavailable
        }
        return playtimes
    }

    func loadAchievements(
        webTokenJSON _: String,
        titleID _: String,
        locale: String
    ) async throws -> [AchievementSummary] {
        await recorder?.recordAchievementRequest(locale: locale)
        return achievements
    }
}

@MainActor
private final class MockPreferredGameLocaleProvider: PreferredGameLocaleProviding {
    let preferredGameLocale: String

    init(preferredGameLocale: String) {
        self.preferredGameLocale = preferredGameLocale
    }
}

private enum MockXboxDataError: Error {
    case unavailable
}

private func readUInt16LE(_ bytes: [UInt8], at offset: Int) -> UInt16 {
    UInt16(bytes[offset]) | UInt16(bytes[offset + 1]) << 8
}

private func readInt16LE(_ bytes: [UInt8], at offset: Int) -> Int16 {
    Int16(bitPattern: readUInt16LE(bytes, at: offset))
}

private enum RecordedPowerCommand: Equatable, Sendable {
    case powerOn(consoleID: String)
    case powerOff(consoleID: String)
}

private actor XboxDataClientRecorder {
    private var achievementRequests = 0
    private var recordedAchievementLocales: [String] = []
    private var hostRequests = 0
    private var libraryRequests = 0
    private var recordedPowerCommands: [RecordedPowerCommand] = []

    func recordHostRequest() {
        hostRequests += 1
    }

    func recordPowerCommand(_ command: RecordedPowerCommand) {
        recordedPowerCommands.append(command)
    }

    func recordAchievementRequest(locale: String) {
        achievementRequests += 1
        recordedAchievementLocales.append(locale)
    }

    func recordLibraryRequest() {
        libraryRequests += 1
    }

    func achievementRequestCount() -> Int {
        achievementRequests
    }

    func achievementLocales() -> [String] {
        recordedAchievementLocales
    }

    func hostRequestCount() -> Int {
        hostRequests
    }

    func libraryRequestCount() -> Int {
        libraryRequests
    }

    func powerCommands() -> [RecordedPowerCommand] {
        recordedPowerCommands
    }
}

@MainActor
private final class MockWebAuthentication: WebAuthenticating {
    private let callbackURL: URL
    private(set) var lastPrefersEphemeralSession: Bool?

    init(
        callbackURL: URL = URL(string: "ms-xal-000000004c20a908://auth/?code=code")!
    ) {
        self.callbackURL = callbackURL
    }

    func authenticate(
        authorizationURL _: String,
        prefersEphemeralSession: Bool
    ) async throws -> URL {
        lastPrefersEphemeralSession = prefersEphemeralSession
        return callbackURL
    }

    func cancel() {}
}
