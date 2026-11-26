import AVFoundation
import Combine
import CoreVideo
import Metal
import SwiftUI
import UIKit
@preconcurrency import WebRTC
import XCTest

@testable import XBXRC

private struct StreamingNativeFrameDelivery: @unchecked Sendable {
    let renderer: any RTCVideoRenderer
    let frame: RTCVideoFrame

    func deliver() {
        renderer.setSize(CGSize(width: 64, height: 64))
        renderer.renderFrame(frame)
    }
}

@MainActor
private final class StreamingPublicationRecorder {
    var states: [(UInt64, StreamingFeatureState)] = []
    var context: StreamingPresentationTraceContext?
}

final class StreamingDisplayCallbackTests: XCTestCase {
    private func makeFrame(_ mailbox: StreamingLatestFrameMailbox) throws
        -> StreamingDecodedFrameRef
    {
        var buffer: CVPixelBuffer?
        XCTAssertEqual(
            CVPixelBufferCreate(
                kCFAllocatorDefault, 16, 16, kCVPixelFormatType_32BGRA, nil, &buffer),
            kCVReturnSuccess)
        return mailbox.makeFrame(
            storage: .pixelBuffer(try XCTUnwrap(buffer)), width: 16, height: 16,
            rotationDegrees: 0, isFullRange: true, traceContext: nil)
    }

    @MainActor
    func testGpuCompletionReleasesSlotBeforeMainActorAccountingAndKeepsNewBusyFact() async throws {
        let mailbox = StreamingLatestFrameMailbox()
        let telemetry = StreamingDisplayTelemetry()
        let first = try makeFrame(mailbox)
        let second = try makeFrame(mailbox)
        _ = mailbox.submit(first)
        XCTAssertEqual(mailbox.beginRender().frame?.identifier, first.identifier)
        _ = telemetry.recordSubmit(identifier: first.identifier, timestampMilliseconds: 100)
        _ = mailbox.submit(second)
        let hasPending = await Task.detached {
            mailbox.finishRender(identifier: first.identifier, requeueAsLatest: false)
        }.value
        XCTAssertTrue(hasPending)
        XCTAssertFalse(mailbox.snapshotDrawableBusy())
        // 主线程尚未记账第一帧时，下一次显示 tick 已可取得第二帧。
        XCTAssertEqual(mailbox.beginRender().frame?.identifier, second.identifier)
        let oldCompletion = telemetry.recordPresent(
            identifier: first.identifier, timestampMilliseconds: 102,
            drawableBusy: mailbox.snapshotDrawableBusy())
        XCTAssertTrue(oldCompletion.snapshot.currentDrawableBusy)
        XCTAssertEqual(oldCompletion.snapshot.submitToPresentP95Milliseconds, 2)
        _ = mailbox.finishRender(identifier: second.identifier, requeueAsLatest: false)
    }

    func testLateAndDuplicateCompletionKeepResetFrameOwnedByNewSubmission() throws {
        let mailbox = StreamingLatestFrameMailbox()
        let old = try makeFrame(mailbox)
        _ = mailbox.submit(old)
        _ = mailbox.beginRender()
        mailbox.reset()
        let current = try makeFrame(mailbox)
        XCTAssertGreaterThan(current.identifier, old.identifier)
        _ = mailbox.submit(current)
        _ = mailbox.beginRender()
        for _ in 0..<2 {
            _ = mailbox.finishRender(identifier: old.identifier, requeueAsLatest: false)
            XCTAssertTrue(mailbox.snapshotDrawableBusy())
            XCTAssertTrue(mailbox.beginRender().drawableBusy)
        }
        _ = mailbox.finishRender(identifier: current.identifier, requeueAsLatest: false)
        XCTAssertFalse(mailbox.snapshotDrawableBusy())
    }

    func testFailedCompletionRetriesFrameAndPrefersNewerCandidate() throws {
        let mailbox = StreamingLatestFrameMailbox()
        let first = try makeFrame(mailbox)
        _ = mailbox.submit(first)
        _ = mailbox.beginRender()
        XCTAssertTrue(mailbox.finishRender(identifier: first.identifier, requeueAsLatest: true))
        XCTAssertEqual(mailbox.beginRender().frame?.identifier, first.identifier)
        let second = try makeFrame(mailbox)
        _ = mailbox.submit(second)
        XCTAssertTrue(mailbox.finishRender(identifier: first.identifier, requeueAsLatest: true))
        XCTAssertEqual(mailbox.snapshotOverwriteCount(), 1)
        XCTAssertEqual(mailbox.beginRender().frame?.identifier, second.identifier)
    }

    func testHealthBurstPreservesLatestCountersAndFlushesTailOnce() throws {
        let context = StreamingPresentationTraceContext(
            attemptID: "burst", generation: 1, peerEpoch: 1)
        var coalescer = StreamingDisplayHealthCoalescer()
        XCTAssertNotNil(coalescer.offer(context: context, snapshot: .zero))
        let telemetry = StreamingDisplayTelemetry()
        var latest = StreamingDisplayHealthSnapshot.zero
        for identifier in 1...600 {
            _ = telemetry.recordSubmit(
                identifier: UInt64(identifier), timestampMilliseconds: Double(identifier))
            latest =
                telemetry.recordPresent(
                    identifier: UInt64(identifier), timestampMilliseconds: Double(identifier) + 0.5
                ).snapshot
            XCTAssertNil(coalescer.offer(context: context, snapshot: latest))
        }
        guard case .displayHealth(let observedContext, let snapshot) = coalescer.flush() else {
            return XCTFail("缺少尾部健康快照")
        }
        XCTAssertEqual(observedContext, context)
        XCTAssertEqual(snapshot.submitCount, 600)
        XCTAssertEqual(snapshot.presentCount, 600)
        XCTAssertEqual(snapshot.latencySampleCount, 240)
        XCTAssertEqual(snapshot, latest)
        XCTAssertNil(coalescer.flush())
        XCTAssertNil(coalescer.offer(context: context, snapshot: latest))
        XCTAssertNil(coalescer.pending)
    }

    func testHealthContextChangeAndResetDiscardOldPendingSnapshot() {
        let first = StreamingPresentationTraceContext(
            attemptID: "first", generation: 1, peerEpoch: 1)
        let second = StreamingPresentationTraceContext(
            attemptID: "second", generation: 2, peerEpoch: 2)
        var coalescer = StreamingDisplayHealthCoalescer()
        _ = coalescer.offer(context: first, snapshot: .zero)
        let busy = StreamingDisplayTelemetry().setDrawableBusy(true)
        _ = coalescer.offer(context: first, snapshot: busy)
        guard
            case .displayHealth(let context, let snapshot) = coalescer.offer(
                context: second, snapshot: .zero)
        else {
            return XCTFail("新代次应立即报告健康基线")
        }
        XCTAssertEqual(context, second)
        XCTAssertEqual(snapshot, .zero)
        XCTAssertNil(coalescer.flush())
        _ = coalescer.offer(context: second, snapshot: busy)
        coalescer.reset()
        XCTAssertNil(coalescer.flush())
        XCTAssertNotNil(coalescer.offer(context: second, snapshot: .zero))
    }

    func testCompletionTimingSeparatesQueueAndWorkAndSurvivesRedaction() {
        let timing = StreamingGpuCompletionTiming(
            completedAtMs: 100, slotReleasedAtMs: 100.1, mainActorEnteredAtMs: 150,
            feedbackFinishedAtMs: 151, telemetryFinishedAtMs: 153)
        let payload = timing.tracePayload(recordedAtMs: 154)
        XCTAssertEqual(payload["gpuToMainActorMs"], .double(50))
        XCTAssertEqual(payload["frameFeedbackWorkMs"], .double(1))
        XCTAssertEqual(payload["displayTelemetryWorkMs"], .double(2))
        XCTAssertEqual(payload["mainActorWorkBeforeTraceMs"], .double(4))
        XCTAssertEqual(payload["gpuToTraceRecordMs"], .double(54))
        for profile in [IOSRuntimeTraceProfile.dev, .production] {
            XCTAssertEqual(
                IOSRuntimeTraceRedactor.redact(
                    payload: payload, profile: profile, fingerprintSalt: "test"),
                payload)
        }
    }

    func testIngressDrainContinuesWithinDeliveryAndYieldsAfterTwoFrames() throws {
        let slot = StreamingLatestIngressSlot<Int>()
        let firstTicket = try XCTUnwrap(slot.submit(1, scheduledAtMs: 100))
        var delivered: [Int] = []
        let nextTicket = try XCTUnwrap(
            slot.drain(firstTicket) { delivery in
                delivered.append(delivery.value)
                XCTAssertNil(slot.submit(delivery.value + 1))
            })
        XCTAssertEqual(delivered, [1, 2])
        XCTAssertGreaterThan(nextTicket.identifier, firstTicket.identifier)
        XCTAssertNil(slot.drain(firstTicket) { _ in XCTFail("旧任务应保持接续帧归属") })
        XCTAssertNil(slot.drain(nextTicket) { delivered.append($0.value) })
        XCTAssertEqual(delivered, [1, 2, 3])
        XCTAssertNotNil(slot.submit(4))
    }

    func testIngressDrainKeepsLatestReplacementCountsAndResetBoundary() throws {
        let slot = StreamingLatestIngressSlot<Int>()
        let ticket = try XCTUnwrap(slot.submit(1))
        var delivered: [(Int, Int64)] = []
        XCTAssertNil(
            slot.drain(ticket) { delivery in
                delivered.append((delivery.value, delivery.replacedCount))
                if delivery.value == 1 {
                    _ = slot.submit(2)
                    _ = slot.submit(3)
                }
            })
        XCTAssertEqual(delivered.map(\.0), [1, 3])
        XCTAssertEqual(delivered.map(\.1), [0, 1])
        let oldTicket = try XCTUnwrap(slot.submit(4))
        slot.reset()
        XCTAssertNil(slot.drain(oldTicket) { _ in XCTFail("旧入口帧应在reset边界释放") })
        XCTAssertNotNil(slot.submit(5))
    }

    func testIngressReplacementKeepsOriginalDrainScheduleTime() throws {
        let slot = StreamingLatestIngressSlot<Int>()
        let ticket = try XCTUnwrap(slot.submit(1, scheduledAtMs: 100))
        for value in 2...39 {
            XCTAssertNil(slot.submit(value, scheduledAtMs: Double(100 + value)))
        }
        XCTAssertEqual(ticket.scheduledAtMs, 100)
        var delivered: StreamingLatestIngressDelivery<Int>?
        XCTAssertNil(slot.drain(ticket) { delivered = $0 })
        XCTAssertEqual(delivered?.value, 39)
        XCTAssertEqual(delivered?.replacedCount, 38)
    }

    func testIngressStaleAndDuplicateTasksKeepConcurrentResetFramesOwnedByNewTicket() async throws {
        let slot = StreamingLatestIngressSlot<Int>()
        let oldTicket = try XCTUnwrap(slot.submit(1, scheduledAtMs: 100))
        let scheduledAfterReset = await Task.detached {
            slot.reset()
            return slot.submit(2, scheduledAtMs: 200)
        }.value
        let newTicket = try XCTUnwrap(scheduledAfterReset)
        XCTAssertNil(slot.submit(3, scheduledAtMs: 201))
        XCTAssertGreaterThan(newTicket.identifier, oldTicket.identifier)
        for _ in 0..<2 {
            XCTAssertNil(slot.drain(oldTicket) { _ in XCTFail("迟到任务应保持新代次帧归属") })
            XCTAssertNil(slot.take(oldTicket))
            XCTAssertNil(slot.finishDelivery(oldTicket))
        }
        var delivered: StreamingLatestIngressDelivery<Int>?
        XCTAssertNil(slot.drain(newTicket) { delivered = $0 })
        XCTAssertEqual(delivered?.value, 3)
        XCTAssertEqual(delivered?.replacedCount, 1)
        let latest = try XCTUnwrap(slot.submit(4))
        XCTAssertNil(slot.drain(newTicket) { _ in XCTFail("重复任务应保持最新帧归属") })
        XCTAssertEqual(slot.take(latest)?.value, 4)
        XCTAssertNil(slot.finishDelivery(latest))
    }

    func testIngressResetDuringConsumptionKeepsNewScheduleAndReplacementCount() throws {
        let slot = StreamingLatestIngressSlot<Int>()
        let oldTicket = try XCTUnwrap(slot.submit(1))
        var newTicket: StreamingIngressDrainTicket?
        XCTAssertNil(
            slot.drain(oldTicket) { delivery in
                XCTAssertEqual(delivery.value, 1)
                slot.reset()
                newTicket = slot.submit(2)
                XCTAssertNil(slot.submit(3))
            })
        let currentTicket = try XCTUnwrap(newTicket)
        var delivered: StreamingLatestIngressDelivery<Int>?
        XCTAssertNil(slot.drain(currentTicket) { delivered = $0 })
        XCTAssertEqual(delivered?.value, 3)
        XCTAssertEqual(delivered?.replacedCount, 1)
    }

    func testIngressAndDrawPreparationTimingSurviveRedaction() {
        let timing = StreamingFrameIngressTiming(
            receivedAtMs: 119, drainScheduledAtMs: 100, drainStartedAtMs: 120,
            mainActorEnteredAtMs: 121)
        var payload = timing.tracePayload(acceptedAtMs: 123)
        XCTAssertEqual(payload["ingressQueueMs"], .double(2))
        XCTAssertEqual(payload["ingressDrainQueueMs"], .double(20))
        XCTAssertEqual(payload["ingressDrainWorkBeforeFrameMs"], .double(1))
        XCTAssertEqual(payload["frameAcceptWorkMs"], .double(2))
        var dropPayload = timing.queueTracePayload()
        dropPayload["count"] = .integer(38)
        dropPayload["reason"] = .string("ingressCoalesced")
        for key in timing.queueTracePayload().keys {
            XCTAssertEqual(payload[key], dropPayload[key])
        }
        payload["drawToSubmitMs"] = .double(4)
        payload["drawableAcquireMs"] = .double(2)
        payload["presentationActive"] = .bool(false)
        for profile in [IOSRuntimeTraceProfile.dev, .production] {
            XCTAssertEqual(
                IOSRuntimeTraceRedactor.redact(
                    payload: payload, profile: profile, fingerprintSalt: "test"), payload)
            XCTAssertEqual(
                IOSRuntimeTraceRedactor.redact(
                    payload: dropPayload, profile: profile, fingerprintSalt: "test"), dropPayload)
        }
    }

    @MainActor
    func testTouchOverlayKeepsBindingAcrossDiagnosticsAndReleasesOnSuppression() async throws {
        let suite = "XBXRC.StreamingTouchSuppression.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        let settings = AppSettingsStore(defaults: defaults)
        let session = StreamInputSession(controller: { nil })
        _ = session.outputState(now: 0)
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.first as? UIWindowScene)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 800, height: 420)
        let hosting = UIHostingController(
            rootView: TouchGamepadOverlay(input: session).environmentObject(settings))
        window.rootViewController = hosting
        window.isHidden = false
        defer {
            window.isHidden = true
            window.rootViewController = nil
            defaults.removePersistentDomain(forName: suite)
        }
        func findSurface(_ view: UIView) -> TouchLayoutSurfaceView? {
            if let surface = view as? TouchLayoutSurfaceView { return surface }
            for child in view.subviews {
                if let surface = findSurface(child) { return surface }
            }
            return nil
        }
        hosting.view.layoutIfNeeded()
        for _ in 0..<50 {
            if let surface = findSurface(hosting.view), surface.input.onInput != nil,
                !surface.input.sockets.isEmpty
            {
                break
            }
            try await Task.sleep(for: .milliseconds(20))
        }
        let surface = try XCTUnwrap(findSurface(hosting.view))
        let a = try XCTUnwrap(
            surface.input.sockets.first { $0.control.action == .gamepad("gamepadA") })
        surface.input.process([.init(id: 1, phase: .began, position: a.center)])
        XCTAssertEqual(session.outputState(now: 1).buttonMask, 16)
        for _ in 0..<20 { session.refreshDeviceStatus() }
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertTrue(findSurface(hosting.view) === surface)
        XCTAssertEqual(surface.input.gamepad.buttonMask, 16)
        let point = surface.convert(a.center, to: window)
        XCTAssertTrue(window.hitTest(point, with: nil) === surface)
        session.setSuppressed(true)
        for _ in 0..<50 {
            if surface.input.gamepad == .neutral, window.hitTest(point, with: nil) !== surface {
                break
            }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertEqual(surface.input.gamepad, .neutral)
        XCTAssertEqual(session.outputState(now: 2), .neutral)
        XCTAssertFalse(window.hitTest(point, with: nil) === surface)
        session.setSuppressed(false)
        for _ in 0..<50 {
            if window.hitTest(point, with: nil) === surface { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(window.hitTest(point, with: nil) === surface)
        XCTAssertEqual(session.outputState(now: 3), .neutral)
        surface.input.process([.init(id: 2, phase: .began, position: a.center)])
        XCTAssertEqual(session.outputState(now: 4).buttonMask, 16)
        surface.stop()
        XCTAssertEqual(session.outputState(now: 5), .neutral)
    }

    func testReceiverTrackWrappersKeepNativeIdentityAcrossRepeatedDiscovery() throws {
        RTCInitializeSSL()
        let factory = RTCPeerConnectionFactory()
        let configuration = RTCConfiguration()
        configuration.sdpSemantics = .unifiedPlan
        let peer = try XCTUnwrap(
            factory.peerConnection(
                with: configuration,
                constraints: RTCMediaConstraints(
                    mandatoryConstraints: nil, optionalConstraints: nil),
                delegate: nil))
        defer { peer.close() }
        let transceiverInit = RTCRtpTransceiverInit()
        transceiverInit.direction = .recvOnly
        let transceiver = try XCTUnwrap(peer.addTransceiver(of: .video, init: transceiverInit))
        let first = try XCTUnwrap(transceiver.receiver.track as? RTCVideoTrack)
        for _ in 0..<60 {
            let next = try XCTUnwrap(transceiver.receiver.track as? RTCVideoTrack)
            XCTAssertFalse(first === next)
            XCTAssertTrue(streamingVideoTracksMatch(first, next))
        }
    }

    func testDifferentNativeTracksWithSameIdentifierRequireReattachment() {
        let factory = RTCPeerConnectionFactory()
        let first = factory.videoTrack(with: factory.videoSource(), trackId: "shared-label")
        let replacement = factory.videoTrack(with: factory.videoSource(), trackId: "shared-label")
        XCTAssertEqual(first.trackId, replacement.trackId)
        XCTAssertFalse(streamingVideoTracksMatch(first, replacement))
        XCTAssertTrue(streamingVideoTracksMatch(first, first))
        XCTAssertTrue(streamingVideoTracksMatch(nil, nil))
        XCTAssertFalse(streamingVideoTracksMatch(first, nil))
        XCTAssertFalse(streamingVideoTracksMatch(nil, replacement))
    }

    private func makeNativeFrame() throws -> RTCVideoFrame {
        var buffer: CVPixelBuffer?
        let attributes =
            [
                kCVPixelBufferMetalCompatibilityKey: true,
                kCVPixelBufferIOSurfacePropertiesKey: [:],
            ] as CFDictionary
        XCTAssertEqual(
            CVPixelBufferCreate(
                kCFAllocatorDefault, 64, 64, kCVPixelFormatType_32BGRA, attributes, &buffer),
            kCVReturnSuccess)
        return RTCVideoFrame(
            buffer: RTCCVPixelBuffer(pixelBuffer: try XCTUnwrap(buffer)),
            rotation: ._0, timeStampNs: 0)
    }

    @MainActor
    func testBackgroundPipelineReadinessReplaysOncePerVisibleInterval() async throws {
        let surface = StreamingMetalVideoSurfaceView(
            frame: CGRect(x: 0, y: 0, width: 64, height: 64),
            device: try XCTUnwrap(MTLCreateSystemDefaultDevice()))
        defer { surface.shutdown() }
        let context = StreamingPresentationTraceContext(
            attemptID: "background-readiness", generation: 1, peerEpoch: 1)
        var readiness: [StreamingPresentationTraceContext] = []
        var accepted = 0
        surface.presentationEventSink = { event in
            if case .displayPipelineReady(let context) = event { readiness.append(context) }
            if case .firstFrameAccepted = event { accepted += 1 }
        }
        surface.setPresentationActive(false)
        surface.updateTraceContext(context)
        let delivery = StreamingNativeFrameDelivery(
            renderer: surface.renderer, frame: try makeNativeFrame())
        await Task.detached { delivery.deliver() }.value
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertTrue(readiness.isEmpty)
        XCTAssertEqual(accepted, 0)
        surface.setPresentationActive(true)
        XCTAssertEqual(readiness, [context])
        surface.setPresentationActive(true)
        surface.updateTraceContext(context)
        XCTAssertEqual(readiness, [context])
        surface.setPresentationActive(false)
        surface.setPresentationActive(true)
        XCTAssertEqual(readiness, [context, context])
        XCTAssertEqual(accepted, 0)
    }

    @MainActor
    func testPipelineReadinessWaitsForSinkAndDiscardsReplacedContext() async throws {
        let surface = StreamingMetalVideoSurfaceView(
            frame: CGRect(x: 0, y: 0, width: 64, height: 64),
            device: try XCTUnwrap(MTLCreateSystemDefaultDevice()))
        defer { surface.shutdown() }
        surface.setPresentationActive(false)
        let first = StreamingPresentationTraceContext(
            attemptID: "first", generation: 1, peerEpoch: 1)
        let current = StreamingPresentationTraceContext(
            attemptID: "current", generation: 2, peerEpoch: 2)
        let delivery = StreamingNativeFrameDelivery(
            renderer: surface.renderer, frame: try makeNativeFrame())
        for context in [first, current] {
            surface.updateTraceContext(context)
            await Task.detached { delivery.deliver() }.value
            try await Task.sleep(for: .milliseconds(100))
        }
        surface.setPresentationActive(true)
        var readiness: [StreamingPresentationTraceContext] = []
        let sink: @MainActor (StreamingPresentationLifecycleEvent) -> Void = { event in
            if case .displayPipelineReady(let context) = event { readiness.append(context) }
        }
        surface.presentationEventSink = sink
        XCTAssertEqual(readiness, [current])
        surface.presentationEventSink = sink
        XCTAssertEqual(readiness, [current])
    }

    @MainActor
    private func startReadySession(
        resumeGate: StreamingAsyncGate? = nil, failResume: Bool = false,
        snapshotGate: StreamingAsyncGate? = nil, presentsInitialFrame: Bool = true
    ) async throws -> (
        StreamSessionActor, StreamingPublicationRecorder, StreamingRuntimeRecorder
    ) {
        let recorder = StreamingRuntimeRecorder()
        let publication = StreamingPublicationRecorder()
        let session = StreamSessionActor(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(
                    recorder: recorder, resumeValidationGate: resumeGate,
                    failResumeValidation: failResume), recorder: recorder),
            peerFactory: StreamingTestPeerFactory(
                recorder: recorder, debugSnapshotGate: snapshotGate))
        try await session.start(
            request: StreamingLaunchRequest(
                streamTitleID: "test-title", accessHandle: "test-access",
                accountGeneration: 1, sessionGeneration: 1),
            stateSink: { publication.states.append(($0, $1)) },
            videoSink: { _, track in publication.context = track?.traceContext },
            diagnosticsSink: { _, _ in })
        let deadline = ContinuousClock.now.advanced(by: .seconds(2))
        while publication.context == nil {
            guard ContinuousClock.now < deadline else { throw StreamingRuntimeError.superseded }
            try await Task.sleep(for: .milliseconds(10))
        }
        let context = try XCTUnwrap(publication.context)
        await session.displayPipelineReady(context: context)
        await session.firstFrameAcceptedToMailbox(context: context)
        await session.firstFrameSubmitted(context: context)
        if presentsInitialFrame {
            await session.firstFramePresented(context: context)
            XCTAssertEqual(publication.states.last?.1, .playing)
        } else {
            XCTAssertEqual(publication.states.last?.1, .waitingForFirstFrame)
        }
        return (session, publication, recorder)
    }

    private func presentSnapshot() -> StreamingDisplayHealthSnapshot {
        let telemetry = StreamingDisplayTelemetry()
        let now = StreamingFrameFeedbackClock.nowMilliseconds
        _ = telemetry.recordSubmit(identifier: 1, timestampMilliseconds: now)
        return telemetry.recordPresent(identifier: 1, timestampMilliseconds: now).snapshot
    }

    @MainActor
    func testBackgroundPresentationRetainsPlayingOnOriginalPeer() async throws {
        let (session, publication, recorder) = try await startReadySession(failResume: true)
        defer { Task { await session.stop(generation: 1) } }
        let context = try XCTUnwrap(publication.context)
        let count = publication.states.count
        await session.setVisible(false, generation: 1)
        XCTAssertEqual(publication.states.last?.1, .playing)
        await session.setVisible(true, generation: 1)
        await session.displayHealthUpdated(context: context, snapshot: presentSnapshot())
        XCTAssertEqual(publication.states.count, count)
        XCTAssertEqual(publication.context, context)
        let snapshot = await recorder.snapshot()
        let validations = await recorder.resumeValidationCount()
        XCTAssertEqual(validations, 0)
        XCTAssertEqual(snapshot.offers, ["local-offer"])
        XCTAssertEqual(snapshot.iceRestartOffers, [false])
        XCTAssertEqual(snapshot.peerStops, 0)
        XCTAssertEqual(snapshot.controlCloses, 0)
        XCTAssertEqual(snapshot.releasedAccessHandles, [])
        await session.stop(generation: 1)
    }

    @MainActor
    func testRepeatedVisibilityChangesKeepPresentationFacts() async throws {
        let (session, publication, recorder) = try await startReadySession()
        defer { Task { await session.stop(generation: 1) } }
        let context = try XCTUnwrap(publication.context)
        let old = presentSnapshot()
        let count = publication.states.count
        for _ in 0..<3 {
            await session.setVisible(false, generation: 1)
            await session.setVisible(false, generation: 1)
            await session.displayHealthUpdated(context: context, snapshot: old)
            await session.setVisible(true, generation: 1)
            await session.setVisible(true, generation: 1)
            await session.displayHealthUpdated(context: context, snapshot: presentSnapshot())
        }
        XCTAssertEqual(publication.states.count, count)
        XCTAssertEqual(publication.states.last?.1, .playing)
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.offers, ["local-offer"])
        await session.stop(generation: 1)
    }

    @MainActor
    func testBackgroundStartupContinuesOnOriginalPeerUntilFreshPresent() async throws {
        let (session, publication, recorder) = try await startReadySession(
            presentsInitialFrame: false)
        defer { Task { await session.stop(generation: 1) } }
        let context = try XCTUnwrap(publication.context)
        await session.setVisible(false, generation: 1)
        await session.firstFramePresented(context: context)
        await session.displayHealthUpdated(context: context, snapshot: presentSnapshot())
        XCTAssertEqual(publication.states.last?.1, .waitingForFirstFrame)
        await session.setVisible(true, generation: 1)
        await session.displayHealthUpdated(context: context, snapshot: presentSnapshot())
        XCTAssertEqual(publication.states.last?.1, .playing)
        XCTAssertEqual(publication.context, context)
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.offers, ["local-offer"])
        await session.stop(generation: 1)
    }

    @MainActor
    func testForegroundPresentationContinuesWhilePeerSnapshotIsBusy() async throws {
        let gate = StreamingAsyncGate()
        await gate.open()
        let (session, publication, _) = try await startReadySession(snapshotGate: gate)
        defer {
            Task {
                await gate.open()
                await session.stop(generation: 1)
            }
        }
        await gate.close()
        await session.setVisible(false, generation: 1)
        let continued = expectation(description: "原连接立即续接呈现")
        let foreground = Task {
            await session.setVisible(true, generation: 1)
            continued.fulfill()
        }
        await fulfillment(of: [continued], timeout: 1)
        XCTAssertEqual(publication.states.last?.1, .playing)
        await gate.open()
        await foreground.value
        await session.stop(generation: 1)
    }

    @MainActor
    func testLateVisibilityAndPresentationAfterStopKeepSessionClosed() async throws {
        let (session, publication, recorder) = try await startReadySession()
        let context = try XCTUnwrap(publication.context)
        await session.stop(generation: 1)
        let count = publication.states.count
        for visible in [false, true, false] {
            await session.setVisible(visible, generation: 1)
            await session.displayHealthUpdated(context: context, snapshot: presentSnapshot())
        }
        XCTAssertEqual(publication.states.last?.1, .idle)
        XCTAssertEqual(publication.states.count, count)
        XCTAssertEqual(publication.states.filter { $0.1 == .playing }.count, 1)
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.peerStops, 1)
        XCTAssertEqual(snapshot.controlCloses, 1)
        XCTAssertEqual(snapshot.offers, ["local-offer"])
    }

    func testSuspensionCancelsPendingTimingWhilePreservingCumulativeCounters() {
        let telemetry = StreamingDisplayTelemetry()
        _ = telemetry.setDrawableBusy(true)
        _ = telemetry.recordSubmit(identifier: 1, timestampMilliseconds: 100)
        let snapshot = telemetry.cancelPendingSubmissions()
        XCTAssertEqual(snapshot.submitCount, 1)
        XCTAssertFalse(snapshot.currentDrawableBusy)
        _ = telemetry.recordPresent(identifier: 1, timestampMilliseconds: 101)
        _ = telemetry.recordSubmit(identifier: 2, timestampMilliseconds: 200)
        let resumed = telemetry.recordPresent(identifier: 2, timestampMilliseconds: 202).snapshot
        XCTAssertEqual(resumed.submitCount, 2)
        XCTAssertEqual(resumed.presentCount, 2)
        XCTAssertEqual(resumed.latencySampleCount, 1)
        XCTAssertEqual(resumed.submitToPresentP95Milliseconds, 2)
    }

    @MainActor
    func testActorPublishesStateChangesAcrossHealthBurstResumeAndNewSession() async throws {
        let recorder = StreamingRuntimeRecorder()
        let publication = StreamingPublicationRecorder()
        let session = StreamSessionActor(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder), recorder: recorder),
            peerFactory: StreamingTestPeerFactory(recorder: recorder))
        for generation: UInt64 in 1...2 {
            try await session.start(
                request: StreamingLaunchRequest(
                    streamTitleID: "test-title", accessHandle: "test-access",
                    accountGeneration: 1, sessionGeneration: generation),
                stateSink: { publication.states.append(($0, $1)) },
                videoSink: { _, track in publication.context = track?.traceContext },
                diagnosticsSink: { _, _ in })
            let deadline = ContinuousClock.now.advanced(by: .seconds(2))
            while publication.context?.generation != generation {
                guard ContinuousClock.now < deadline else { return XCTFail("缺少当前视频代次") }
                try await Task.sleep(for: .milliseconds(10))
            }
            let context = try XCTUnwrap(publication.context)
            await session.displayPipelineReady(context: context)
            await session.firstFrameAcceptedToMailbox(context: context)
            await session.firstFrameSubmitted(context: context)
            await session.firstFramePresented(context: context)
            for _ in 0..<600 {
                await session.displayHealthUpdated(context: context, snapshot: .zero)
            }
            XCTAssertEqual(
                publication.states.filter { $0.0 == generation && $0.1 == .playing }.count, 1)
            await session.setVisible(false, generation: generation)
            await session.setVisible(false, generation: generation)
            XCTAssertEqual(
                publication.states.filter { $0.0 == generation && $0.1 == .suspending }.count, 0)
            await session.setVisible(true, generation: generation)
            let now = StreamingFrameFeedbackClock.nowMilliseconds
            let telemetry = StreamingDisplayTelemetry()
            _ = telemetry.recordSubmit(identifier: 1, timestampMilliseconds: now)
            let fresh = telemetry.recordPresent(identifier: 1, timestampMilliseconds: now).snapshot
            await session.displayHealthUpdated(context: context, snapshot: fresh)
            XCTAssertEqual(
                publication.states.filter { $0.0 == generation && $0.1 == .playing }.count, 1)
            await session.stop(generation: generation)
            XCTAssertEqual(
                publication.states.filter { $0.0 == generation && $0.1 == .idle }.count, 1)
        }
    }
}

final class XBXRCTests: XCTestCase {
    @MainActor
    func testNativeVideoRendererPresentsNV12FrameDeliveredFromBackgroundQueue() async throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.first as? UIWindowScene)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 320, height: 180)
        let controller = UIViewController()
        window.rootViewController = controller
        let device = try XCTUnwrap(MTLCreateSystemDefaultDevice())
        let surface = StreamingMetalVideoSurfaceView(frame: window.bounds, device: device)
        controller.view.addSubview(surface)
        let context = StreamingPresentationTraceContext(
            attemptID: UUID().uuidString, generation: 1, peerEpoch: 1)
        surface.updateTraceContext(context)
        var didPresent = false
        var healthReports: [(Double, StreamingDisplayHealthSnapshot)] = []
        surface.presentationEventSink = { event in
            if case .firstFramePresented(let observed) = event, observed == context {
                didPresent = true
            }
            if case .displayHealth(let observed, let snapshot) = event, observed == context {
                healthReports.append((StreamingFrameFeedbackClock.nowMilliseconds, snapshot))
            }
        }
        window.isHidden = false
        controller.view.layoutIfNeeded()
        defer {
            surface.shutdown()
            window.isHidden = true
        }
        var pixelBuffer: CVPixelBuffer?
        let attributes =
            [
                kCVPixelBufferMetalCompatibilityKey: true,
                kCVPixelBufferIOSurfacePropertiesKey: [:],
            ] as CFDictionary
        XCTAssertEqual(
            CVPixelBufferCreate(
                kCFAllocatorDefault, 64, 64, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                attributes, &pixelBuffer), kCVReturnSuccess)
        let buffer = try XCTUnwrap(pixelBuffer)
        CVPixelBufferLockBaseAddress(buffer, [])
        for plane in 0..<CVPixelBufferGetPlaneCount(buffer) {
            if let bytes = CVPixelBufferGetBaseAddressOfPlane(buffer, plane) {
                memset(
                    bytes, plane == 0 ? 16 : 128,
                    CVPixelBufferGetBytesPerRowOfPlane(buffer, plane)
                        * CVPixelBufferGetHeightOfPlane(buffer, plane))
            }
        }
        CVPixelBufferUnlockBaseAddress(buffer, [])
        let delivery = StreamingNativeFrameDelivery(
            renderer: surface.renderer,
            frame: RTCVideoFrame(
                buffer: RTCCVPixelBuffer(pixelBuffer: buffer),
                rotation: ._0, timeStampNs: 0))
        await Task.detached { delivery.deliver() }.value
        for _ in 0..<150 {
            if didPresent { break }
            try await Task.sleep(for: .milliseconds(20))
        }
        XCTAssertTrue(didPresent)
        healthReports.removeAll()
        for _ in 0..<30 {
            await Task.detached { delivery.deliver() }.value
            try await Task.sleep(for: .milliseconds(20))
        }
        try await Task.sleep(for: .milliseconds(200))
        let tail = try XCTUnwrap(healthReports.last?.1)
        XCTAssertGreaterThan(tail.presentCount, 10)
        XCTAssertEqual(tail.submitCount, tail.presentCount)
        XCTAssertLessThan(healthReports.count, Int(tail.presentCount))
        for (previous, next) in zip(healthReports, healthReports.dropFirst()) {
            XCTAssertGreaterThanOrEqual(next.0 - previous.0, 90)
        }
    }

    func testSdpDiagnosticSummaryKeepsPayloadsScopedToMediaSections() {
        // PC 现有协商样本在两个 media section 中复用 payload 96。
        let sdp = """
            v=0
            m=audio 9 RTP/SAVPF 96
            a=rtpmap:96 opus/48000/2
            m=video 9 RTP/SAVPF 96 97
            a=rtpmap:96 H264/90000
            a=rtpmap:97 rtx/90000
            a=fmtp:96 profile-level-id=42e01f;packetization-mode=1
            m=application 9 UDP/DTLS/SCTP webrtc-datachannel
            """
        guard case .object(let summary) = StreamingRuntimeDiagnostics.sdpSummaryPayload(sdp) else {
            return XCTFail("missing SDP summary")
        }
        XCTAssertEqual(summary["audioCodecs"], .array([.string("opus")]))
        XCTAssertEqual(summary["videoCodecs"], .array([.string("H264"), .string("rtx")]))
    }

    func testSdpDiagnosticSummaryHandlesRepeatedMappingsAndMissingMedia() {
        let sdp = "m=audio 9 RTP/SAVPF 111\na=rtpmap:111 opus/48000/2\na=rtpmap:111 opus/48000/2\n"
        guard case .object(let summary) = StreamingRuntimeDiagnostics.sdpSummaryPayload(sdp) else {
            return XCTFail("missing SDP summary")
        }
        XCTAssertEqual(summary["audioCodecs"], .array([.string("opus")]))
        XCTAssertEqual(summary["videoCodecs"], .array([]))
    }

    func testNativeWebRtcInitialOfferUsesFeedbackFactoryInBothModes() async throws {
        for preventResolutionDrops in [false, true] {
            let runtime = LibWebRTCPeerRuntimeFactory().makeRuntime(eventSink: { _ in })
            do {
                let signaling = StreamingPreparedSignaling(
                    iceServers: [],
                    webRtcPlan: StreamingWebRtcPlan(
                        preventResolutionDrops: preventResolutionDrops)
                )
                let offer = try await runtime.makeOffer(configuration: signaling, iceRestart: false)
                XCTAssertTrue(offer.contains("m=audio "))
                XCTAssertTrue(offer.contains("m=video "))
                XCTAssertTrue(offer.contains("H264/90000"))
                XCTAssertTrue(offer.contains("m=application "))
                let audioSession = RTCAudioSession.sharedInstance()
                XCTAssertTrue(audioSession.useManualAudio)
                XCTAssertTrue(audioSession.isAudioEnabled)
                // 直接验证默认 ADM 初始化会检查的系统输入路径。
                XCTAssertTrue(audioSession.inputAvailable)
                XCTAssertGreaterThan(audioSession.inputNumberOfChannels, 0)
            } catch {
                await runtime.closeTransport()
                throw error
            }
            await runtime.closeTransport()
            XCTAssertFalse(RTCAudioSession.sharedInstance().isAudioEnabled)
        }
    }

    @MainActor
    func testAppOrientationGeometryTargetsStreamingAndApplicationModes() {
        XCTAssertEqual(
            XBXRCAppDelegate.requestedGeometryMask(for: .phone, mode: .streaming),
            .landscape
        )
        XCTAssertEqual(
            XBXRCAppDelegate.requestedGeometryMask(for: .phone, mode: .application),
            .portrait
        )
        XCTAssertEqual(
            XBXRCAppDelegate.requestedGeometryMask(for: .pad, mode: .application),
            .all
        )
    }

    func testStreamingPlaybackReadinessRequiresAllPresentationFacts() {
        var readiness = StreamingPlaybackReadiness()
        readiness.mark(.peerConnected)
        readiness.mark(.controlReady)
        readiness.mark(.displayPipelineReady)
        readiness.mark(.frameAcceptedToMailbox)
        readiness.mark(.frameSubmitted)

        XCTAssertFalse(readiness.isPlaybackReady)
        XCTAssertTrue(readiness.mark(.framePresented))
        XCTAssertTrue(readiness.isPlaybackReady)
        XCTAssertFalse(readiness.mark(.framePresented))

        readiness.remove(.peerConnected)
        XCTAssertFalse(readiness.isPlaybackReady)
        readiness.resetForPeer()
        XCTAssertTrue(readiness.facts.isEmpty)
    }

    func testStreamingCleanupStagesKeepPresentationFirstAndAccessLast() {
        XCTAssertEqual(StreamingCleanupStage.allCases.first, .presentationDetached)
        XCTAssertEqual(StreamingCleanupStage.allCases.last, .accessReleased)
    }

    func testStreamingLatestIngressSlotCoalescesPendingFramesAndReschedules() throws {
        let slot = StreamingLatestIngressSlot<Int>()

        let firstTicket = try XCTUnwrap(slot.submit(1, scheduledAtMs: 100))
        XCTAssertNil(slot.submit(2, scheduledAtMs: 101))
        XCTAssertNil(slot.submit(3, scheduledAtMs: 102))
        let firstDelivery = slot.take(firstTicket)
        XCTAssertEqual(firstDelivery?.value, 3)
        XCTAssertEqual(firstDelivery?.replacedCount, 2)

        XCTAssertNil(slot.submit(4, scheduledAtMs: 103))
        let nextTicket = try XCTUnwrap(slot.finishDelivery(firstTicket, scheduledAtMs: 110))
        XCTAssertGreaterThan(nextTicket.identifier, firstTicket.identifier)
        XCTAssertEqual(nextTicket.scheduledAtMs, 110)
        XCTAssertNil(slot.take(firstTicket))
        XCTAssertNil(slot.finishDelivery(firstTicket))
        let secondDelivery = slot.take(nextTicket)
        XCTAssertEqual(secondDelivery?.value, 4)
        XCTAssertEqual(secondDelivery?.replacedCount, 0)
        XCTAssertNil(slot.finishDelivery(nextTicket))

        let oldTicket = try XCTUnwrap(slot.submit(5))
        slot.reset()
        XCTAssertNil(slot.take(oldTicket))
        XCTAssertNotNil(slot.submit(6))
    }

    func testStreamingRenderRetryWaitsForDisplayTickWhenFrameIsRequeued() {
        XCTAssertFalse(
            StreamingRenderRetryPolicy.shouldRequestImmediateTick(
                requeueAsLatest: true,
                hasPendingFrame: true
            )
        )
        XCTAssertTrue(
            StreamingRenderRetryPolicy.shouldRequestImmediateTick(
                requeueAsLatest: false,
                hasPendingFrame: true
            )
        )
        XCTAssertFalse(
            StreamingRenderRetryPolicy.shouldRequestImmediateTick(
                requeueAsLatest: false,
                hasPendingFrame: false
            )
        )
    }

    func testStreamingMailboxRequeueKeepsNewerCandidate() {
        XCTAssertTrue(
            StreamingMailboxRequeuePolicy.shouldRestoreInflight(
                hasLatestCandidate: false
            )
        )
        XCTAssertFalse(
            StreamingMailboxRequeuePolicy.shouldRestoreInflight(
                hasLatestCandidate: true
            )
        )
    }

    func testStreamingPresentationContextChangeDetectionIsValueBased() {
        let baseline = StreamingPresentationTraceContext(
            attemptID: "attempt-1",
            generation: 3,
            peerEpoch: 7
        )

        XCTAssertFalse(
            streamingPresentationContextDidChange(previous: nil, next: nil)
        )
        XCTAssertTrue(
            streamingPresentationContextDidChange(previous: nil, next: baseline)
        )
        XCTAssertTrue(
            streamingPresentationContextDidChange(previous: baseline, next: nil)
        )
        XCTAssertFalse(
            streamingPresentationContextDidChange(previous: baseline, next: baseline)
        )
        XCTAssertTrue(
            streamingPresentationContextDidChange(
                previous: baseline,
                next: StreamingPresentationTraceContext(
                    attemptID: "attempt-2",
                    generation: baseline.generation,
                    peerEpoch: baseline.peerEpoch
                )
            )
        )
        XCTAssertTrue(
            streamingPresentationContextDidChange(
                previous: baseline,
                next: StreamingPresentationTraceContext(
                    attemptID: baseline.attemptID,
                    generation: baseline.generation &+ 1,
                    peerEpoch: baseline.peerEpoch
                )
            )
        )
        XCTAssertTrue(
            streamingPresentationContextDidChange(
                previous: baseline,
                next: StreamingPresentationTraceContext(
                    attemptID: baseline.attemptID,
                    generation: baseline.generation,
                    peerEpoch: baseline.peerEpoch &+ 1
                )
            )
        )
    }

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

    func testStreamingCodecPreferencePolicyUsesReceiverCapabilitiesForReceiveOnlyMedia() {
        XCTAssertEqual(
            StreamingCodecPreferencePolicy.capabilityRole(for: .receiveOnly),
            .receiver
        )
        XCTAssertEqual(
            StreamingCodecPreferencePolicy.capabilityRole(for: .sendReceive),
            .sender
        )
        XCTAssertLessThan(
            StreamingCodecPreferencePolicy.videoCodecRank("H264"),
            StreamingCodecPreferencePolicy.videoCodecRank("VP8")
        )
        XCTAssertLessThan(
            StreamingCodecPreferencePolicy.audioCodecRank("opus"),
            StreamingCodecPreferencePolicy.audioCodecRank("PCMU")
        )
    }

    func testStreamingStatsRequestGateIsSingleFlightAndRejectsStaleCallbacks() {
        var gate = StreamingStatsRequestGate()
        let firstGeneration = gate.activate()

        XCTAssertTrue(gate.begin(generation: firstGeneration))
        XCTAssertFalse(gate.begin(generation: firstGeneration))
        XCTAssertTrue(gate.complete(generation: firstGeneration))
        XCTAssertTrue(gate.begin(generation: firstGeneration))

        gate.invalidate()
        let secondGeneration = gate.activate()
        XCTAssertTrue(gate.begin(generation: secondGeneration))
        XCTAssertFalse(gate.complete(generation: firstGeneration))
        XCTAssertTrue(gate.requestInFlight)
        XCTAssertTrue(gate.complete(generation: secondGeneration))
        XCTAssertFalse(gate.requestInFlight)
    }

    func testIOSStreamInputPacketMatchesXboxGamepadWireLayout() {
        let packet = IOSStreamInputPacketEncoder.encodeGamepad(
            sequence: 0x0102_0304,
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

        XCTAssertEqual(
            bytes,
            [
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
        XCTAssertEqual(Array(bytes[0..<6]), [2, 0, 4, 3, 2, 1])
        XCTAssertEqual(bytes[14], 1)
        XCTAssertEqual(bytes[15], 0)
        XCTAssertEqual(readUInt16LE(bytes, at: 16), 0xA55A)
        XCTAssertEqual(readInt16LE(bytes, at: 18), 16_384)
        XCTAssertEqual(readInt16LE(bytes, at: 20), -16_384)
        XCTAssertEqual(readInt16LE(bytes, at: 22), -32_767)
        XCTAssertEqual(readInt16LE(bytes, at: 24), 32_767)
        XCTAssertEqual(readUInt16LE(bytes, at: 26), 16_384)
        XCTAssertEqual(readUInt16LE(bytes, at: 28), 65_535)
        XCTAssertEqual(Array(bytes[30..<38]), Array(repeating: UInt8(0), count: 8))
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

        XCTAssertFalse(
            gate.shouldSend(
                state: .neutral,
                bufferedAmount: IOSStreamInputSendGate.highWatermarkBytes,
                nowMilliseconds: 300
            ))
        XCTAssertTrue(gate.blocked)
        XCTAssertFalse(
            gate.shouldSend(
                state: .neutral,
                bufferedAmount: IOSStreamInputSendGate.lowWatermarkBytes + 1,
                nowMilliseconds: 310
            ))
        XCTAssertTrue(
            gate.shouldSend(
                state: .neutral,
                bufferedAmount: IOSStreamInputSendGate.lowWatermarkBytes,
                nowMilliseconds: 320
            ))
        XCTAssertFalse(gate.blocked)
    }

    func testIOSStreamInputDrainPolicyWaitsForLowWatermarkAndHasFixedUpperBound() {
        let policy = IOSStreamInputDrainPolicy.standard

        XCTAssertFalse(
            policy.shouldEnqueueNeutral(
                bufferedAmount: policy.lowWatermarkBytes + 1,
                elapsedMilliseconds: policy.maximumWaitMilliseconds - 1
            ))
        XCTAssertTrue(
            policy.shouldEnqueueNeutral(
                bufferedAmount: policy.lowWatermarkBytes,
                elapsedMilliseconds: 0
            ))
        XCTAssertTrue(
            policy.shouldEnqueueNeutral(
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
        XCTAssertEqual(
            fourRoutes.map(\.route),
            [
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
        XCTAssertEqual(minimumDuration.first?.durationMilliseconds, 1)

        let stop = IOSStreamRumbleEffect(
            gamepadIndex: 0,
            strongMagnitude: 1,
            weakMagnitude: 1,
            leftTrigger: 1,
            rightTrigger: 1,
            durationMilliseconds: 0,
            format: .betterXcloud
        )
        XCTAssertTrue(
            IOSStreamHapticsRouter.pulses(
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
                )
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
            XCTAssertNil(
                StreamingNativeIceCandidateAdapter.adapt(
                    StreamingIceCandidate(
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
                Data(
                    #"{"type":"Message","target":"/streaming/characteristics/dimensionschanged"}"#
                        .utf8)
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
        XCTAssertEqual(
            LibraryPresentation.heroGames(from: games).map(\.id),
            [
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
            "Game Pass 新入库 · Action"
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

    func testCloudLibraryDiagnosticsRetainsRetryableQueryNetworkFailure() {
        let error = XboxBridgeError.Query(
            failure: XboxQueryFailure(
                kind: .network, statusCode: nil, retriable: true, retryAfterSeconds: nil))
        let payload = CloudLibraryDiagnostics.errorPayload(error, extra: ["phase": "loaded"])

        XCTAssertEqual(payload["errorKind"], .string("network"))
        XCTAssertEqual(payload["retriable"], .bool(true))
        XCTAssertEqual(payload["timeout"], .bool(false))
        XCTAssertEqual(payload["phase"], .string("loaded"))
        XCTAssertNil(payload["statusCode"])
    }

    func testCloudLibraryDiagnosticsRetainsQueryStatusAndCooldown() {
        let error = XboxBridgeError.Query(
            failure: XboxQueryFailure(
                kind: .rateLimited, statusCode: 429, retriable: true, retryAfterSeconds: 30))
        let payload = CloudLibraryDiagnostics.errorPayload(error)

        XCTAssertEqual(payload["errorKind"], .string("rateLimited"))
        XCTAssertEqual(payload["statusCode"], .integer(429))
        XCTAssertEqual(payload["retryAfterSeconds"], .integer(30))
        XCTAssertEqual(payload["retriable"], .bool(true))
        XCTAssertEqual(payload["timeout"], .bool(false))
    }

    func testCloudLibraryDiagnosticsRetainsQueryTimeoutAndPermanentNetworkFailure() {
        let timeout = XboxBridgeError.Query(
            failure: XboxQueryFailure(
                kind: .timeout, statusCode: nil, retriable: true, retryAfterSeconds: nil))
        let timeoutPayload = CloudLibraryDiagnostics.errorPayload(timeout)
        XCTAssertEqual(timeoutPayload["errorKind"], .string("timeout"))
        XCTAssertEqual(timeoutPayload["timeout"], .bool(true))
        XCTAssertEqual(timeoutPayload["retriable"], .bool(true))

        let permanent = XboxBridgeError.Query(
            failure: XboxQueryFailure(
                kind: .network, statusCode: nil, retriable: false, retryAfterSeconds: nil))
        XCTAssertEqual(
            CloudLibraryDiagnostics.errorPayload(permanent)["retriable"], .bool(false))
    }

    func testAuthStageDiagnosticsRetainsNetworkFailureDetails() {
        for (kind, requestStage, elapsed, failureKind) in [
            ("connect", "deviceToken", UInt64(156), XboxQueryFailureKind.network),
            ("timeout", "sisuAuthorize", UInt64(5_011), XboxQueryFailureKind.timeout),
        ] {
            let event = XboxAuthStage(
                stage: "authRequestFailed", requestStage: requestStage, scope: "cloud",
                elapsedMs: elapsed, skipCoreRefresh: nil, networkErrorKind: kind,
                failure: XboxQueryFailure(
                    kind: failureKind, statusCode: nil, retriable: true, retryAfterSeconds: nil))
            let payload = RustAuthCheckpointObserver.stagePayload(event)

            XCTAssertEqual(payload["networkErrorKind"], .string(kind))
            XCTAssertEqual(payload["requestStage"], .string(requestStage))
            XCTAssertEqual(payload["elapsedMs"], .integer(Int64(elapsed)))
            XCTAssertEqual(payload["scope"], .string("cloud"))
            XCTAssertEqual(payload["errorKind"], .string(String(describing: failureKind)))
            XCTAssertEqual(payload["retriable"], .bool(true))
        }
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
            budgetOverride: IOSRuntimeTraceBudget(maxFileBytes: 1_800, maxFiles: 2)
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
        XCTAssertTrue(fileSizes.allSatisfy { $0 <= 1_800 })
        XCTAssertFalse(envelopes.isEmpty)
        XCTAssertTrue(envelopes.contains { $0.event == "sample" })
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
            language: CloudLibraryStore.catalogLanguage(
                preferredLanguage: Locale.preferredLanguages.first),
            market: "US"
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
        let profileSourceRequests = await recorder.profileSourceRequestCounts()
        XCTAssertEqual(
            profileSourceRequests, [.identity: 1, .social: 1, .friends: 1, .presence: 1])
        for source in profileSourceRequests.keys {
            XCTAssertEqual(store.profileSectionStates[source]?.phase, .loaded)
        }
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
            store.hostPowerCommandState
                == .executing(
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
    func testXboxDataStoreKeepsLibraryWhenPlaytimeFails() async throws {
        let store = XboxDataStore(
            client: MockXboxDataClient(games: [Self.game], failsPlaytime: true)
        )

        await store.sync(session: Self.storedSession)
        await store.activateLibraryOnce()

        XCTAssertEqual(store.libraryPhase, .loaded)
        XCTAssertEqual(store.games, [Self.game])
        XCTAssertNotNil(store.libraryUpdatedAt)
        XCTAssertNil(store.libraryFailure)
        XCTAssertNil(store.libraryErrorMessage)
        let playtimeState = try XCTUnwrap(store.historyPlaytimeStates["self"])
        XCTAssertEqual(playtimeState.phase, .failed)
        XCTAssertEqual(playtimeState.failure?.kind, .unknown)
        XCTAssertFalse(playtimeState.isRefreshing)
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

        store.start(streamTitleID: "  1234abcd  ") { _ in
            Self.streamingAccess(handle: "access-1")
        }

        try await waitUntil { store.videoTrack?.traceContext != nil }
        try await waitUntil { store.state == .waitingForFirstFrame }
        let currentContext = try XCTUnwrap(store.videoTrack?.traceContext)
        store.handlePresentationEvent(
            .displayPipelineReady(
                StreamingPresentationTraceContext(
                    attemptID: currentContext.attemptID,
                    generation: currentContext.generation,
                    peerEpoch: currentContext.peerEpoch &+ 1
                )
            )
        )
        try await Task.sleep(for: .milliseconds(20))
        XCTAssertEqual(store.state, .waitingForFirstFrame)
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }
        try await waitUntilAsync {
            (await recorder.snapshot()).remoteCandidatesApplied.count == 1
        }
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.streamTitleIDs, ["1234abcd"])
        XCTAssertEqual(snapshot.offers, ["local-offer"])
        XCTAssertEqual(snapshot.answersApplied, ["remote-answer"])
        XCTAssertEqual(
            snapshot.localCandidates,
            [
                StreamingIceCandidate(
                    sdp: "candidate:local",
                    sdpMid: "0",
                    sdpMLineIndex: 0
                )
            ])
        XCTAssertEqual(
            snapshot.remoteCandidatesApplied,
            [
                StreamingIceCandidate(
                    sdp: "candidate:remote",
                    sdpMid: "0",
                    sdpMLineIndex: 0
                )
            ])
        XCTAssertEqual(snapshot.markConnectedCalls, 1)
        XCTAssertEqual(snapshot.localIceCompletions, 1)
        XCTAssertNotNil(store.videoTrack)
        let traceContext = try XCTUnwrap(store.videoTrack?.traceContext)
        XCTAssertFalse(traceContext.attemptID.isEmpty)
        XCTAssertGreaterThan(traceContext.generation, 0)
        XCTAssertGreaterThan(traceContext.peerEpoch, 0)
        var notifications: [StreamingFeatureState] = []
        let observation = store.$state.sink { notifications.append($0) }
        for _ in 0..<600 {
            store.handlePresentationEvent(.displayHealth(traceContext, .zero))
        }
        try await Task.sleep(for: .milliseconds(150))
        XCTAssertEqual(notifications, [.playing])
        observation.cancel()
        store.stop()
        try await waitUntil { store.state == .idle }
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

        store.start(streamTitleID: "1234abcd") { _ in
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

        store.start(streamTitleID: "1234abcd") { _ in
            Self.streamingAccess(handle: "access-answer-timeout")
        }

        try await waitUntil {
            if case .failed(let message, let retryable) = store.state {
                return retryable
                    && message
                        == StreamingRuntimeError.remoteDescriptionTimedOut.localizedDescription
            }
            return false
        }
        // 终态先解除画面归属，资源清理随后完成；等待最后一步再核对释放次数。
        try await waitUntilAsync {
            (await recorder.snapshot()).releasedAccessHandles == ["access-answer-timeout"]
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

        store.start(streamTitleID: "1234abcd") { _ in
            throw NSError(
                domain: "stream-access-fixture",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "access unavailable"]
            )
        }

        try await waitUntil {
            if case .failed(_, let retryable) = store.state {
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
            "m=video 9 UDP/TLS/RTP/SAVPF 96 97 98 99",
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
        XCTAssertTrue(
            output.contains(
                "a=fmtp:97 profile-level-id=4d0032;packetization-mode=1;level-asymmetry-allowed=1;max-fs=3600;max-fr=60;x-google-min-bitrate=4000;x-google-start-bitrate=8000;x-google-max-bitrate=12000"
            ))
        XCTAssertTrue(output.contains("a=fmtp:111 minptime=10;useinbandfec=1;stereo=1"))
        XCTAssertTrue(output.contains("a=rtcp-fb:97 transport-cc"))
        XCTAssertTrue(output.contains("a=rtcp-fb:97 nack pli"))
        XCTAssertTrue(output.contains("a=fmtp:99 apt=97;rtx-time=200"))
    }

    func testStreamingPreparedSignalingDefaultsToReceiveOnlyAudio() {
        let signaling = StreamingPreparedSignaling(iceServers: [])

        XCTAssertEqual(signaling.webRtcPlan.audioDirection, .receiveOnly)
        XCTAssertEqual(signaling.webRtcPlan.videoDirection, .receiveOnly)
        XCTAssertEqual(signaling.webRtcPlan.h264PacketizationMode, 1)
        XCTAssertEqual(signaling.webRtcPlan.h264Profiles, ["4d", "42e", "420"])
    }

    func testStreamingBackgroundAudioIsDeclared() {
        XCTAssertEqual(
            Bundle.main.object(forInfoDictionaryKey: "UIBackgroundModes") as? [String], ["audio"])
    }

    func testStreamingAudioPermissionOnlyAppliesToSendReceive() {
        XCTAssertFalse(
            StreamingAudioPermissionCoordinator.requiresMicrophonePermission(for: .receiveOnly)
        )
        XCTAssertTrue(
            StreamingAudioPermissionCoordinator.requiresMicrophonePermission(for: .sendReceive)
        )
    }

    func testStreamingAudioConfigurationsAreIndependentAndDirectionComplete() {
        let receiveOnly = StreamingAudioLifecycleCoordinator.makeConfiguration(
            for: .receiveOnly
        )
        let sendReceive = StreamingAudioLifecycleCoordinator.makeConfiguration(
            for: .sendReceive
        )

        XCTAssertFalse(receiveOnly === sendReceive)
        XCTAssertEqual(receiveOnly.category, AVAudioSession.Category.playAndRecord.rawValue)
        XCTAssertEqual(receiveOnly.mode, AVAudioSession.Mode.voiceChat.rawValue)
        XCTAssertEqual(
            receiveOnly.categoryOptions,
            [.defaultToSpeaker, .allowBluetoothHFP, .allowBluetoothA2DP, .allowAirPlay]
        )
        XCTAssertEqual(sendReceive.category, AVAudioSession.Category.playAndRecord.rawValue)
        XCTAssertEqual(sendReceive.mode, AVAudioSession.Mode.voiceChat.rawValue)
        XCTAssertEqual(
            sendReceive.categoryOptions,
            [.defaultToSpeaker, .allowBluetoothHFP, .allowBluetoothA2DP, .allowAirPlay]
        )
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
    func testStreamingFailedPerformsIceRestartWithinSameSession() async throws {
        let recorder = StreamingRuntimeRecorder()
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder),
                recorder: recorder
            ),
            peerFactory: StreamingTestPeerFactory(
                recorder: recorder,
                terminalStateAfterConnect: .failed
            )
        )

        store.start(streamTitleID: "1234abcd") { _ in
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
            terminalStateAfterConnect: .failed,
            failIceRestart: true
        )
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder),
                recorder: recorder
            ),
            peerFactory: peerFactory
        )

        store.start(streamTitleID: "1234abcd") { _ in
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
            terminalStateAfterConnect: .failed,
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

        store.start(streamTitleID: "1234abcd") { _ in
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
        XCTAssertEqual(
            snapshot.cleanupEvents,
            [
                "inputStopped", "hapticsStopped", "peerClosed", "remoteSessionClosed",
                "accessReleased",
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
                terminalStateAfterConnect: .failed
            )
        )

        store.start(streamTitleID: "1234abcd") { _ in
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
                && current.localIceCompletions == 2
                && current.markConnectedCalls >= 1
        }
        snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.localCandidateSubmitStarts, 2)
        XCTAssertEqual(snapshot.localCandidateSubmitFinishes, 2)
        XCTAssertEqual(snapshot.localIceCompletions, 2)
        // 协商期间的 connected 可由初次连接或恢复连接确认，旧候选提交始终完成后再开始新协商。
        XCTAssertGreaterThanOrEqual(snapshot.markConnectedCalls, 1)
    }

    @MainActor
    func testHomeTurnFallbackUsesDirectFirstThenOneReplacement() async throws {
        let recorder = StreamingRuntimeRecorder()
        let stun = StreamingIceServer(
            urls: ["stun:fixture.invalid:3478"], username: nil, credential: nil)
        let turn = StreamingIceServer(
            urls: ["turn:fixture.invalid:3478"], username: "fixture", credential: "fixture")
        let prepared = StreamingPreparedSignaling(
            iceServers: [stun], fallbackIceServers: [stun, turn])
        let factory = StreamingTestPeerFactory(recorder: recorder, connectsFirstRuntime: false)
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder, prepared: prepared),
                recorder: recorder),
            peerFactory: factory, initialConnectionTimeout: .milliseconds(30))
        store.startHome(targetID: "fixture-console") { _ in
            Self.streamingHomeAccess(handle: "fallback-access")
        }
        try await waitUntilAsync {
            let snapshot = await recorder.snapshot()
            return factory.runtimesCreated == 2 && snapshot.markConnectedCalls == 1
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }
        let counts = await recorder.peerIceServerCounts()
        XCTAssertEqual(counts, [1, 2])
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.iceRestartOffers, [false, false])
        XCTAssertEqual(snapshot.peerStops, 1)
        XCTAssertEqual(snapshot.controlCloses, 0)
        store.stop()
        try await waitUntil { store.state == .idle }
    }

    @MainActor
    func testInitialConnectionDeadlineWithoutFallbackClosesOnce() async throws {
        let recorder = StreamingRuntimeRecorder()
        let factory = StreamingTestPeerFactory(recorder: recorder, connectsFirstRuntime: false)
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder), recorder: recorder),
            peerFactory: factory, initialConnectionTimeout: .milliseconds(30))
        store.start(streamTitleID: "1234abcd") { _ in
            Self.streamingAccess(handle: "deadline-access")
        }
        try await waitUntil { store.canRetry }
        try await waitUntilAsync {
            let snapshot = await recorder.snapshot()
            return snapshot.controlCloses == 1
                && snapshot.releasedAccessHandles == ["deadline-access"]
        }
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(factory.runtimesCreated, 1)
        XCTAssertEqual(snapshot.controlCloses, 1)
        XCTAssertEqual(snapshot.releasedAccessHandles, ["deadline-access"])
        store.stop()
        try await waitUntil { store.state == .idle }
    }

    @MainActor
    func testStreamingTemporaryDisconnectRetainsPeerAndAcceptsAutonomousRecovery() async throws {
        let recorder = StreamingRuntimeRecorder()
        let factory = StreamingTestPeerFactory(
            recorder: recorder, transientDisconnectAfterConnect: true)
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder), recorder: recorder),
            peerFactory: factory)
        store.start(streamTitleID: "1234abcd") { _ in
            Self.streamingAccess(handle: "temporary-disconnect")
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }
        try await Task.sleep(for: .milliseconds(100))
        factory.emitConnection(.connected)
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.iceRestartOffers, [false])
        XCTAssertEqual(snapshot.peerStops, 0)
        XCTAssertEqual(snapshot.controlCloses, 0)
        XCTAssertEqual(store.state, .playing)
        store.stop()
        try await waitUntil { store.state == .idle }
    }

    @MainActor
    func testStreamingFailedAndClosedRecoverWithinRemoteSessionAndCleanupOnce() async throws {
        for terminal in [StreamingPeerConnectionState.failed, .closed] {
            let recorder = StreamingRuntimeRecorder()
            let factory = StreamingTestPeerFactory(
                recorder: recorder, terminalStateAfterConnect: terminal)
            let store = StreamingFeatureStore(
                controlFactory: StreamingTestControlFactory(
                    session: StreamingTestControlSession(recorder: recorder), recorder: recorder),
                peerFactory: factory)
            store.start(streamTitleID: "1234abcd") { _ in
                Self.streamingAccess(handle: "access-\(terminal.rawValue)")
            }
            try await waitUntilAsync {
                await recorder.snapshot().iceRestartOffers == [false, true]
            }
            try await markCurrentVideoSurfaceReady(store)
            try await waitUntil { store.state == .playing }
            // 新帧观察事务期间重复事实共用同一个恢复动作。
            factory.emitConnection(terminal)
            factory.emitConnection(terminal)
            try await Task.sleep(for: .milliseconds(50))
            var snapshot = await recorder.snapshot()
            XCTAssertEqual(snapshot.iceRestartOffers, [false, true])
            XCTAssertEqual(snapshot.controlCloses, 0)
            store.stop()
            try await waitUntil { store.state == .idle }
            snapshot = await recorder.snapshot()
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

        store.startHome(targetID: "  console-server-id  ") { _ in
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

        store.start(streamTitleID: "1234abcd") { _ in
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
        store.start(streamTitleID: "1234abcd") { _ in
            Self.streamingAccess(handle: "access-stop")
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }
        XCTAssertNotNil(store.videoTrack)

        store.stop()
        XCTAssertNil(store.videoTrack)
        store.stop()
        try await waitUntil { store.state == .idle }

        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.peerStops, 1)
        XCTAssertEqual(snapshot.controlCloses, 1)
        XCTAssertEqual(snapshot.releasedAccessHandles, ["access-stop"])
        XCTAssertEqual(
            snapshot.cleanupEvents,
            [
                "inputStopped", "hapticsStopped", "peerClosed", "remoteSessionClosed",
                "accessReleased",
            ])
        XCTAssertNil(store.videoTrack)
    }

    @MainActor
    func testStreamingFeatureStoreBackgroundKeepsPlayingAndOriginalConnection()
        async throws
    {
        let recorder = StreamingRuntimeRecorder()
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(
                    recorder: recorder, failResumeValidation: true),
                recorder: recorder
            ),
            peerFactory: StreamingTestPeerFactory(recorder: recorder)
        )
        store.start(streamTitleID: "1234abcd") { _ in
            Self.streamingAccess(handle: "access-background")
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }

        store.handleScenePhase(.background)
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(store.state, .playing)
        XCTAssertTrue(store.input.isSuppressed)
        XCTAssertFalse(store.presentationActive)

        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.peerStops, 0)
        XCTAssertEqual(snapshot.controlCloses, 0)
        XCTAssertEqual(snapshot.releasedAccessHandles, [])
        let context = try XCTUnwrap(store.videoTrack?.traceContext)
        store.handleScenePhase(.active)
        try await Task.sleep(for: .milliseconds(100))
        XCTAssertEqual(store.state, .playing)
        XCTAssertEqual(store.videoTrack?.traceContext, context)
        let validations = await recorder.resumeValidationCount()
        XCTAssertEqual(validations, 0)
        let continued = await recorder.snapshot()
        XCTAssertEqual(continued.offers, ["local-offer"])
        XCTAssertEqual(continued.iceRestartOffers, [false])
        let now = ProcessInfo.processInfo.systemUptime * 1_000
        store.handlePresentationEvent(
            .displayHealth(
                context,
                StreamingDisplayHealthSnapshot(
                    submitCount: 2, presentCount: 2, lastSubmitAtMilliseconds: now,
                    lastPresentAtMilliseconds: now, submitToPresentP50Milliseconds: 1,
                    submitToPresentP95Milliseconds: 2, displayedFrameDelta: 1,
                    mailboxOverwriteCount: 0,
                    dropCountByReason: [:], currentDrawableBusy: false)))
        try await waitUntil { store.state == .playing }
        XCTAssertTrue(store.presentationActive)
        XCTAssertFalse(store.input.isSuppressed)
        store.stop()
        try await waitUntil { store.state == .idle }
        let closed = await recorder.snapshot()
        XCTAssertEqual(closed.peerStops, 1)
        XCTAssertEqual(closed.controlCloses, 1)
        XCTAssertEqual(closed.releasedAccessHandles, ["access-background"])
    }

    @MainActor
    func testMicrophoneAndMuteRestoreAfterPeerReplacement() async throws {
        let recorder = StreamingRuntimeRecorder()
        let factory = StreamingTestPeerFactory(recorder: recorder, failIceRestart: true)
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder), recorder: recorder),
            peerFactory: factory)
        store.start(streamTitleID: "1234abcd") { _ in Self.streamingAccess(handle: "audio-rebuild")
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }
        store.setAudioMuted(true)
        store.setMicrophoneEnabled(true)
        try await waitUntil { store.microphoneEnabled && !store.microphonePending }
        factory.emitConnection(.failed)
        try await waitUntilAsync {
            let offers = await recorder.snapshot().offers
            return factory.runtimesCreated == 2
                && offers.filter { $0 == "chat:microphone-on" }.count == 2
        }
        try await waitUntil { store.microphoneEnabled && !store.microphonePending }
        let muted = await recorder.audioMutedStates()
        XCTAssertEqual(muted, [false, true, true])
        XCTAssertTrue(store.audioMuted)
        store.stop()
        try await waitUntil { store.state == .idle }
        XCTAssertFalse(store.microphoneEnabled)
    }

    @MainActor
    func testNativeQualityActionConsumesSharedDecisionAndChatKeepsSamePeer() async throws {
        let recorder = StreamingRuntimeRecorder()
        let control = StreamingTestControlSession(recorder: recorder)
        await control.setSupervisionDecision { sample in
            var decision = StreamingSupervisionDecision.unavailable
            decision.fresh = true
            decision.quality = .init(
                state: "congested", reason: "transportLoss",
                action:
                    sample.qualityCompletion == nil
                    ? .init(id: 1, level: 1, reason: "sustainedCongestion") : nil)
            return decision
        }
        let factory = StreamingTestPeerFactory(recorder: recorder)
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(session: control, recorder: recorder),
            peerFactory: factory)
        store.start(streamTitleID: "1234abcd") { _ in
            Self.streamingAccess(handle: "quality-consumer")
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }
        try await waitUntilAsync {
            await recorder.snapshot().offers.contains("quality:quality-15000")
        }
        store.setMicrophoneEnabled(true)
        try await waitUntil { store.microphoneEnabled }
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(
            snapshot.offers, ["local-offer", "quality:quality-15000", "chat:microphone-on"])
        XCTAssertEqual(factory.runtimesCreated, 1)
        XCTAssertEqual(snapshot.iceRestartOffers, [false])
        store.stop()
        try await waitUntil { store.state == .idle }
    }

    @MainActor
    func testFirstFrameGuardRequestsKeyframeAfterFourSeconds() async throws {
        let recorder = StreamingRuntimeRecorder()
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder), recorder: recorder),
            peerFactory: StreamingTestPeerFactory(
                recorder: recorder, emitsFirstVideoFrameAutomatically: false))
        store.start(streamTitleID: "1234abcd") { _ in
            Self.streamingAccess(handle: "first-frame-guard")
        }
        try await waitUntil { store.state == .waitingForFirstFrame }
        try await waitUntilAsync(timeout: .seconds(5)) {
            await recorder.snapshot().offers.contains("keyframeRequest")
        }
        XCTAssertEqual(store.state, .waitingForFirstFrame)
        let offers = await recorder.snapshot().offers
        XCTAssertEqual(offers.filter { $0 == "keyframeRequest" }.count, 1)
        store.stop()
        try await waitUntil { store.state == .idle }
    }

    @MainActor
    func testMicrophoneUsesChatNegotiationAndRetainsTheCurrentPeer() async throws {
        let recorder = StreamingRuntimeRecorder()
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder), recorder: recorder),
            peerFactory: StreamingTestPeerFactory(recorder: recorder))
        store.start(streamTitleID: "1234abcd") { _ in
            Self.streamingAccess(handle: "access-microphone")
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }
        for enabled in [true, false] {
            store.setMicrophoneEnabled(enabled)
            try await waitUntil { !store.microphonePending }
            XCTAssertEqual(store.microphoneEnabled, enabled)
        }
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(
            snapshot.offers, ["local-offer", "chat:microphone-on", "chat:microphone-off"])
        XCTAssertEqual(snapshot.peerStops, 0)
        store.stop()
        try await waitUntil { store.state == .idle }
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
        store.start(streamTitleID: "1111aaaa") { _ in
            Self.streamingAccess(handle: "access-first")
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }

        store.stop()
        store.start(streamTitleID: "2222bbbb") { _ in
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
        let context = try XCTUnwrap(store.videoTrack?.traceContext)
        store.handlePresentationEvent(.displayPipelineReady(context))
        store.handlePresentationEvent(.firstFrameAccepted(context))
        store.handlePresentationEvent(.firstFrameSubmitted(context))
        store.handlePresentationEvent(
            .displayHealth(
                context,
                StreamingDisplayHealthSnapshot(
                    submitCount: 1,
                    presentCount: 1,
                    lastSubmitAtMilliseconds: 1_000,
                    lastPresentAtMilliseconds: 1_016,
                    submitToPresentP50Milliseconds: 16,
                    submitToPresentP95Milliseconds: 16,
                    displayedFrameDelta: 1,
                    mailboxOverwriteCount: 0,
                    dropCountByReason: [:],
                    currentDrawableBusy: false
                )
            )
        )
        store.handlePresentationEvent(.firstFramePresented(context))
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

extension XBXRCTests {
    @MainActor
    func testCloudLaunchUsesFreshlyResolvedStreamIdentity() async throws {
        let recorder = StreamingRuntimeRecorder()
        let resolver = StreamingIdentityTestResolver()
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder), recorder: recorder),
            peerFactory: StreamingTestPeerFactory(recorder: recorder),
            cloudLaunchResolver: resolver
        )
        var authorizationContext: XboxStreamStartup?
        store.startCloudGame(productID: "fixture-product") { startup in
            authorizationContext = startup
            return Self.streamingAccess(handle: "resolved-access")
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }
        let snapshot = await recorder.snapshot()
        let requests = await resolver.requests
        XCTAssertEqual(snapshot.streamTitleIDs, ["resolved-1"])
        XCTAssertEqual(requests.map(\.productID), ["fixture-product"])
        XCTAssertEqual(requests.map(\.accessHandle), ["resolved-access"])
        let controlContexts = await recorder.recordedStartupContexts()
        XCTAssertNotNil(authorizationContext)
        XCTAssertTrue(requests.first?.startup === authorizationContext)
        XCTAssertTrue(controlContexts.first === authorizationContext)
        store.stop()
        try await waitUntil { store.state == .idle }
        let stopped = await recorder.snapshot()
        XCTAssertEqual(stopped.releasedAccessHandles, ["resolved-access"])
    }

    @MainActor
    func testHomeAuthorizationAndControlShareStartupContext() async throws {
        let recorder = StreamingRuntimeRecorder()
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder), recorder: recorder),
            peerFactory: StreamingTestPeerFactory(recorder: recorder))
        var context: XboxStreamStartup?
        store.startHome(targetID: "fixture-console") { startup in
            context = startup
            return PreparedHomeAccess(
                authSession: AuthSession(
                    refreshToken: "fixture", seedJson: "seed",
                    webTokenJson: "web", appLevel: 1), handle: "scoped-home", accountID: "account",
                regionHost: "home.example.com")
        }
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }
        let controlContexts = await recorder.recordedStartupContexts()
        XCTAssertNotNil(context)
        XCTAssertTrue(controlContexts.first === context)
        store.stop()
        try await waitUntil { store.state == .idle }
    }

    @MainActor
    func testStartupFailureRetryabilityReachesPageAndCleanup() async throws {
        for (kind, retryable, status) in [
            (XboxStartupFailureKind.auth, false, UInt16(401)), (.network, true, UInt16(503)),
        ] {
            let recorder = StreamingRuntimeRecorder()
            let failure = XboxStartupFailure(
                kind: kind, phase: "resolvingContext", retryable: retryable,
                statusCode: status, failureCode: nil, networkErrorKind: nil)
            let store = StreamingFeatureStore(
                controlFactory: StreamingTestControlFactory(
                    session: StreamingTestControlSession(
                        recorder: recorder, prepareFailure: failure), recorder: recorder),
                peerFactory: StreamingTestPeerFactory(recorder: recorder))
            store.start(streamTitleID: "1234abcd") { _ in
                Self.streamingAccess(handle: "failed-access")
            }
            try await waitUntil {
                if case .failed = store.state { return true }; return false
            }
            XCTAssertEqual(store.canRetry, retryable)
            let snapshot = await recorder.snapshot()
            XCTAssertEqual(snapshot.controlCloses, 1)
            XCTAssertEqual(snapshot.peerStops, 0)
            XCTAssertEqual(snapshot.releasedAccessHandles, ["failed-access"])
            store.stop()
            try await waitUntil { store.state == .idle }
        }
    }

    @MainActor
    func testCloudLaunchRejectsMissingEntitlementAndInvalidIdentityBeforeSessionCreation()
        async throws
    {
        for invalid in StreamingIdentityTestResolver.InvalidIdentity.allCases {
            let recorder = StreamingRuntimeRecorder()
            let resolver = StreamingIdentityTestResolver(invalid: invalid)
            let store = StreamingFeatureStore(
                controlFactory: StreamingTestControlFactory(
                    session: StreamingTestControlSession(recorder: recorder), recorder: recorder),
                peerFactory: StreamingTestPeerFactory(recorder: recorder),
                cloudLaunchResolver: resolver
            )
            store.startCloudGame(productID: "fixture-product") { _ in
                Self.streamingAccess(handle: "invalid-access")
            }
            try await waitUntil {
                if case .failed = store.state { return true }
                return false
            }
            let snapshot = await recorder.snapshot()
            XCTAssertTrue(snapshot.streamTitleIDs.isEmpty)
            XCTAssertEqual(snapshot.releasedAccessHandles, ["invalid-access"])
            XCTAssertEqual(snapshot.controlCloses, 0)
            store.stop()
            try await waitUntil { store.state == .idle }
        }
    }

    @MainActor
    func testCloudCancellationDuringIdentityLookupReleasesLateAccessThroughInjectedFactory()
        async throws
    {
        let recorder = StreamingRuntimeRecorder()
        let gate = StreamingAsyncGate()
        let resolver = StreamingIdentityTestResolver(gate: gate)
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder), recorder: recorder),
            peerFactory: StreamingTestPeerFactory(recorder: recorder),
            cloudLaunchResolver: resolver
        )
        store.startCloudGame(productID: "fixture-product") { _ in
            Self.streamingAccess(handle: "late-identity-access")
        }
        try await waitUntilAsync { await resolver.requests.count == 1 }
        XCTAssertEqual(store.startupProgress?.phase, "checkingGame")
        store.stop()
        try await waitUntil { store.state == .idle }
        await gate.open()
        try await waitUntilAsync { await recorder.snapshot().releasedAccessHandles.count == 1 }
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.releasedAccessHandles, ["late-identity-access"])
        XCTAssertTrue(snapshot.streamTitleIDs.isEmpty)
        XCTAssertEqual(store.state, .idle)
        XCTAssertNil(store.startupProgress)
    }

    @MainActor
    func testCloudRetryRepeatsIdentityAndRegionResolution() async throws {
        let suite = "XBXRC.CloudIdentityRetry.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = AppSettingsStore(defaults: defaults)
        settings.mobile.cloudServerRegion = "auto"
        let recorder = StreamingRuntimeRecorder()
        let resolver = StreamingIdentityTestResolver(rejectFirst: true)
        let store = StreamingFeatureStore(
            settingsProvider: settings,
            controlFactory: StreamingTestControlFactory(
                session: StreamingTestControlSession(recorder: recorder), recorder: recorder),
            peerFactory: StreamingTestPeerFactory(recorder: recorder),
            cloudLaunchResolver: resolver
        )
        store.startCloudGame(productID: "fixture-product") { _ in
            Self.streamingAccess(handle: "retry-access")
        }
        try await waitUntil { store.canRetry }
        settings.mobile.cloudServerRegion = "westus2"
        store.retry()
        try await markCurrentVideoSurfaceReady(store)
        try await waitUntil { store.state == .playing }
        let requests = await resolver.requests
        XCTAssertEqual(requests.map(\.serverRegion), ["auto", "westus2"])
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.streamTitleIDs, ["resolved-2"])
        XCTAssertEqual(snapshot.releasedAccessHandles, ["retry-access"])
        store.stop()
        try await waitUntil { store.state == .idle }
    }

    @MainActor
    func testStartupQueueProgressIsVisibleAndLateProgressIsDiscardedAfterStop() async throws {
        let recorder = StreamingRuntimeRecorder()
        let gate = StreamingAsyncGate()
        let progress = StreamingStartupProgress(
            phase: "waitingSessionReady", streamState: "Queued",
            totalWaitSeconds: 90, allocationWaitSeconds: 20, provisioningWaitSeconds: 10,
            retryCount: 1,
            retryLimit: 3)
        let control = StreamingTestControlSession(
            recorder: recorder, prepareGate: gate, progress: progress)
        let store = StreamingFeatureStore(
            controlFactory: StreamingTestControlFactory(session: control, recorder: recorder),
            peerFactory: StreamingTestPeerFactory(recorder: recorder)
        )
        store.startHome(targetID: "fixture-console") { _ in
            Self.streamingHomeAccess(handle: "queue-access")
        }
        try await waitUntil { store.startupProgress == progress }
        XCTAssertEqual(store.statusText, "正在排队等待串流会话")
        XCTAssertEqual(store.startupDetailText, "预计等待 90 秒 · 分配 20 秒 · 准备 10 秒 · 重试 1/3")
        store.stop()
        try await waitUntil { store.state == .idle }
        await control.setStartupProgress(StreamingStartupProgress(phase: "ready"))
        await gate.open()
        try await Task.sleep(for: .milliseconds(350))
        XCTAssertNil(store.startupProgress)
        XCTAssertEqual(store.state, .idle)
        let snapshot = await recorder.snapshot()
        XCTAssertEqual(snapshot.releasedAccessHandles, ["queue-access"])
        XCTAssertEqual(snapshot.controlCloses, 1)
    }
}

private actor StreamingIdentityTestResolver: CloudGameLaunchResolving {
    enum InvalidIdentity: CaseIterable {
        case noEntitlement, unknownEntitlement, wrongProduct, emptyTitle
    }
    struct Request: Sendable {
        let accessHandle: String
        let productID: String
        let serverRegion: String
        let startup: XboxStreamStartup?
    }
    private let gate: StreamingAsyncGate?
    private let rejectFirst: Bool
    private let invalid: InvalidIdentity?
    private(set) var requests: [Request] = []
    init(
        gate: StreamingAsyncGate? = nil, rejectFirst: Bool = false, invalid: InvalidIdentity? = nil
    ) {
        self.gate = gate
        self.rejectFirst = rejectFirst
        self.invalid = invalid
    }
    func resolve(
        accessHandle: String, productID: String, serverRegion: String, startup: XboxStreamStartup?
    ) async throws
        -> CloudStreamingIdentity
    {
        requests.append(
            Request(
                accessHandle: accessHandle, productID: productID, serverRegion: serverRegion,
                startup: startup))
        let count = requests.count
        if let gate { await gate.wait() }
        return CloudStreamingIdentity(
            productID: invalid == .wrongProduct ? "another-product" : productID,
            streamTitleID: invalid == .emptyTitle ? "   " : "resolved-\(count)",
            hasEntitlement: invalid == .unknownEntitlement
                ? nil
                : invalid == .noEntitlement || (rejectFirst && count == 1) ? false : true)
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

    func close() { isOpen = false }
    func waiterCount() -> Int { waiters.count }
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
    private var startupContexts: [XboxStreamStartup] = []
    func recordStartupContext(_ context: XboxStreamStartup?) {
        if let context { startupContexts.append(context) }
    }
    func recordedStartupContexts() -> [XboxStreamStartup] { startupContexts }
    private var targets: [StreamingLaunchTarget] = []
    private var streamTitleIDs: [String] = []
    private var settings: [StreamingSessionSettingsSnapshot] = []
    private var offers: [String] = []
    private var mutedStates: [Bool] = []
    private var iceServerCounts: [Int] = []
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
    private var resumeValidationCalls = 0

    func recordResumeValidation() { resumeValidationCalls += 1 }
    func resumeValidationCount() -> Int { resumeValidationCalls }

    func recordTarget(_ value: StreamingLaunchTarget) { targets.append(value) }
    func recordTitleID(_ value: String) { streamTitleIDs.append(value) }
    func recordSettings(_ value: StreamingSessionSettingsSnapshot) { settings.append(value) }
    func recordOffer(_ value: String) { offers.append(value) }
    func recordAudioMuted(_ muted: Bool) { mutedStates.append(muted) }
    func recordIceServers(_ count: Int) { iceServerCounts.append(count) }
    func peerIceServerCounts() -> [Int] { iceServerCounts }
    func audioMutedStates() -> [Bool] { mutedStates }
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

    func createSession(request: StreamingLaunchRequest) async throws -> any StreamingControlSession
    {
        await recorder.recordStartupContext(request.startup)
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
    private let prepareGate: StreamingAsyncGate?
    private let resumeValidationGate: StreamingAsyncGate?
    private let failResumeValidation: Bool
    private let prepareFailure: XboxStartupFailure?
    private var progress: StreamingStartupProgress?
    private var localCandidateSubmitCount = 0
    private let prepared: StreamingPreparedSignaling
    private var recoveryActions = 0
    private var supervisionDecision:
        (@Sendable (StreamingSupervisionSample) -> StreamingSupervisionDecision)?

    func setSupervisionDecision(
        _ decision: @escaping @Sendable (StreamingSupervisionSample) -> StreamingSupervisionDecision
    ) {
        supervisionDecision = decision
    }

    init(
        recorder: StreamingRuntimeRecorder,
        firstLocalCandidateSubmitGate: StreamingAsyncGate? = nil,
        prepareGate: StreamingAsyncGate? = nil,
        resumeValidationGate: StreamingAsyncGate? = nil,
        failResumeValidation: Bool = false,
        prepareFailure: XboxStartupFailure? = nil,
        progress: StreamingStartupProgress? = nil,
        prepared: StreamingPreparedSignaling = .init(iceServers: [])
    ) {
        self.recorder = recorder
        self.firstLocalCandidateSubmitGate = firstLocalCandidateSubmitGate
        self.prepareGate = prepareGate
        self.resumeValidationGate = resumeValidationGate
        self.failResumeValidation = failResumeValidation
        self.prepareFailure = prepareFailure
        self.progress = progress
        self.prepared = prepared
    }

    func prepareSignaling() async throws -> StreamingPreparedSignaling {
        if let prepareFailure { throw XboxStreamingError.Startup(failure: prepareFailure) }
        if let prepareGate { await prepareGate.wait() }
        return prepared
    }

    func exchangeOffer(_ sdp: String) async throws -> String {
        await recorder.recordOffer(sdp)
        return "remote-answer"
    }

    func supervise(_ sample: StreamingSupervisionSample) async throws
        -> StreamingSupervisionDecision
    {
        if let supervisionDecision { return supervisionDecision(sample) }
        var decision = StreamingSupervisionDecision.unavailable
        decision.connectionRecoveryAllowed =
            sample.requestConnectionRecovery
            && !sample.connectionRecoveryActionCompleted && recoveryActions < 2
        if decision.connectionRecoveryAllowed { recoveryActions += 1 }
        decision.connectionRecoveryActive = recoveryActions > 0
        decision.connectionRecoveryExhausted =
            sample.requestConnectionRecovery
            && !sample.connectionRecoveryActionCompleted && recoveryActions >= 2
            && !decision.connectionRecoveryAllowed
        return decision
    }

    func exchangeMediaOffer(_ sdp: String, kind: String) async throws -> String {
        await recorder.recordOffer(kind + ":" + sdp)
        return "remote-answer"
    }

    func startupProgress() async -> StreamingStartupProgress? { progress }
    func setStartupProgress(_ progress: StreamingStartupProgress) { self.progress = progress }

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
                )
            ],
            endOfCandidates: true
        )
    }

    func markConnected() async throws {
        await recorder.recordMarkConnected()
    }

    func validateResume() async throws {
        await recorder.recordResumeValidation()
        if let resumeValidationGate { await resumeValidationGate.wait() }
        if failResumeValidation { throw StreamingRuntimeError.peerConnectionFailed }
    }

    func sendText(_: String) async throws -> Bool { true }

    func powerOffConsole() async throws -> Bool { true }

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
    private let connectsFirstRuntime: Bool
    private let debugSnapshotGate: StreamingAsyncGate?
    private var latestEventSink: (@Sendable (StreamingPeerEvent) -> Void)?

    init(
        recorder: StreamingRuntimeRecorder,
        transientDisconnectAfterConnect: Bool = false,
        terminalStateAfterConnect: StreamingPeerConnectionState? = nil,
        failIceRestart: Bool = false,
        suspendAnswerApply: Bool = false,
        emitsControlReadyAutomatically: Bool = true,
        emitsFirstVideoFrameAutomatically: Bool = true,
        suspendFirstPeerStop: Bool = false,
        connectsFirstRuntime: Bool = true,
        debugSnapshotGate: StreamingAsyncGate? = nil
    ) {
        self.recorder = recorder
        self.connectsFirstRuntime = connectsFirstRuntime
        self.debugSnapshotGate = debugSnapshotGate
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
            stopGate: isFirstRuntime ? firstPeerStopGate : nil,
            emitsConnectedAutomatically: connectsFirstRuntime || !isFirstRuntime,
            debugSnapshotGate: debugSnapshotGate
        )
    }

    func emitConnection(_ state: StreamingPeerConnectionState) {
        latestEventSink?(.connectionStateChanged(state))
    }

    func emitStats(_ stats: StreamingPeerStats) { latestEventSink?(.stats(stats)) }

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
    private var answerCount = 0
    private var qualityCap = 20_000
    private let emitsConnectedAutomatically: Bool
    private let debugSnapshotGate: StreamingAsyncGate?

    init(
        eventSink: @escaping @Sendable (StreamingPeerEvent) -> Void,
        recorder: StreamingRuntimeRecorder,
        transientDisconnectAfterConnect: Bool,
        terminalStateAfterConnect: StreamingPeerConnectionState?,
        failIceRestart: Bool,
        suspendAnswerApply: Bool,
        emitsControlReadyAutomatically: Bool,
        emitsFirstVideoFrameAutomatically: Bool,
        stopGate: StreamingAsyncGate?,
        emitsConnectedAutomatically: Bool = true,
        debugSnapshotGate: StreamingAsyncGate? = nil
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
        self.emitsConnectedAutomatically = emitsConnectedAutomatically
        self.debugSnapshotGate = debugSnapshotGate
    }

    func makeOffer(
        configuration: StreamingPreparedSignaling,
        iceRestart: Bool
    ) async throws -> String {
        offerCount += 1
        await recorder.recordIceServers(configuration.iceServers.count)
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
        answerCount += 1
        if suspendAnswerApply {
            try await Task.sleep(for: .seconds(60))
        }
        if answerCount == 1 {
            eventSink(.dataChannelEvent(label: "all", event: "profilesCreated"))
            eventSink(.dataChannelEvent(label: "message", event: "handshakeAcked"))
            if emitsControlReadyAutomatically {
                eventSink(.dataChannelEvent(label: "control", event: "ready"))
            }
        }
        let trackFactory = RTCPeerConnectionFactory()
        let videoTrack = trackFactory.videoTrack(
            with: trackFactory.videoSource(), trackId: "test-video")
        eventSink(.videoTrack(StreamingVideoTrackHandle(rawValue: videoTrack)))
        eventSink(.audioTrackReady)
        if emitsConnectedAutomatically {
            eventSink(.connectionStateChanged(.connected))
            eventSink(.connectionStateChanged(.connected))
        }
        if emitsFirstVideoFrameAutomatically {
            eventSink(.firstVideoFrame)
        }
        if answerCount == 1, transientDisconnectAfterConnect {
            Task { [eventSink] in
                try? await Task.sleep(for: .milliseconds(1))
                eventSink(.connectionStateChanged(.disconnected))
            }
        } else if answerCount == 1, let terminalStateAfterConnect {
            eventSink(.connectionStateChanged(terminalStateAfterConnect))
            eventSink(.connectionStateChanged(terminalStateAfterConnect))
        }
    }

    func addRemoteCandidates(_ candidates: [StreamingIceCandidate]) async throws {
        await recorder.recordRemoteCandidates(candidates)
    }

    func debugSnapshot() async -> StreamingPeerDebugSnapshot {
        if let debugSnapshotGate { await debugSnapshotGate.wait() }
        return StreamingPeerDebugSnapshot(
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

    func setAudioMuted(_ muted: Bool) async throws { await recorder.recordAudioMuted(muted) }
    func requestVideoKeyframe() async -> Bool {
        await recorder.recordOffer("keyframeRequest")
        return true
    }
    func remoteVideoBitrateCapKbps() async -> Int? { qualityCap }
    func makeQualityOffer(maxVideoBitrateKbps: Int) async throws -> String {
        qualityCap = maxVideoBitrateKbps
        return "quality-\(qualityCap)"
    }
    func makeMicrophoneOffer(enabled: Bool) async throws -> String {
        "microphone-" + (enabled ? "on" : "off")
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
        session: StoredAuthSession, forceRegionIP: String,
        checkpoint: @escaping AuthCredentialCheckpoint,
        authProvider: (any XboxStreamingAuthProvider)?, startup: XboxStreamStartup?
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
        session: StoredAuthSession, forceRegionIP: String,
        checkpoint: @escaping AuthCredentialCheckpoint,
        authProvider: (any XboxStreamingAuthProvider)?, startup: XboxStreamStartup?
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
        session: StoredAuthSession, forceRegionIP: String, scope: XboxAuthScope,
        forceCoreRefresh: Bool, rejectedToken: String?,
        checkpoint: @escaping AuthCredentialCheckpoint
    ) async throws -> AuthSession {
        await recorder?.recordRenew(forceRegionIP)
        return renewedSession
    }

    func loadProfile(webTokenJSON _: String) async throws -> XboxProfile {
        await recorder?.recordProfileRequest()
        return profile
    }

    func loadProfileSource(webTokenJSON _: String, source: XboxProfileSource) async throws
        -> XboxProfile
    {
        await recorder?.recordProfileRequest(source: source)
        return profile
    }
}

private actor RegionRoutingRecorder {
    private var finishRegionIP: String?
    private var renewRegionIP: String?
    private var profileRequests = 0
    private var profileSourceRequests: [XboxProfileSource: Int] = [:]

    func recordFinish(_ value: String) {
        finishRegionIP = value
    }

    func recordRenew(_ value: String) {
        renewRegionIP = value
    }

    func recordProfileRequest(source: XboxProfileSource? = nil) {
        profileRequests += 1
        if let source { profileSourceRequests[source, default: 0] += 1 }
    }

    func lastRenewRegionIP() -> String? {
        renewRegionIP
    }

    func profileRequestCount() -> Int {
        profileRequests
    }

    func profileSourceRequestCounts() -> [XboxProfileSource: Int] {
        profileSourceRequests
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
        return games
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
