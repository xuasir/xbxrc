import CoreVideo
import Foundation
@preconcurrency import WebRTC
import XCTest

@testable import XBXRC

final class StreamingFrameFeedbackTests: XCTestCase {
    func testNativeH264DecoderForwardsFirstFrameFromBackgroundQueue() async throws {
        // 64×64 黑帧的 Main-profile SPS/PPS/IDR，直接经过实际 WebRTC 默认解码器。
        let bitstream = try XCTUnwrap(Data(base64Encoded:
            "AAAAAWdNQArcQmwEQAAAAwBAAAAPI8SJ4AAAAAFo7g/IAAAAAWWIhDr+7c/gUy16TQLf1ODf4Vp+exvZHrCXFw=="))
        let factory = RTCDefaultVideoDecoderFactory()
        let codec = try XCTUnwrap(factory.supportedCodecs().first { $0.name == "H264" })
        for enabled in [false, true] {
            let session = StreamingFrameFeedbackSession(preventResolutionDrops: enabled)
            let decoder = StreamingFeedbackVideoDecoder(
                base: try XCTUnwrap(factory.createDecoder(codec)), feedback: session)
            let firstFrame = expectation(description: "native H264 first frame \(enabled)")
            let queue = DispatchQueue(label: "com.xbxrc.tests.native-h264-decode")
            let decodeResult: Int = await withCheckedContinuation { continuation in
                queue.async {
                    decoder.setCallback { frame in
                        XCTAssertEqual(frame.width, 64)
                        XCTAssertEqual(frame.height, 64)
                        XCTAssertEqual(UInt32(bitPattern: frame.timeStamp), UInt32.max)
                        XCTAssertFalse(Thread.isMainThread)
                        firstFrame.fulfill()
                    }
                    let result = decoder.startDecode(withNumberOfCores: 2)
                    guard result == 0 else {
                        continuation.resume(returning: result)
                        return
                    }
                    let image = RTCEncodedImage()
                    image.buffer = bitstream
                    image.encodedWidth = 64
                    image.encodedHeight = 64
                    image.timeStamp = UInt32.max
                    image.frameType = .videoFrameKey
                    continuation.resume(returning: decoder.decode(
                        image, missingFrames: false, codecSpecificInfo: nil, renderTimeMs: 0))
                }
            }
            XCTAssertEqual(decodeResult, 0)
            await fulfillment(of: [firstFrame], timeout: 5)
            XCTAssertNotNil(session.takeDecodedTiming(
                serverDataKey: UInt32.max,
                nowMilliseconds: StreamingFrameFeedbackClock.nowMilliseconds))
            let releaseResult: Int = await withCheckedContinuation { continuation in
                queue.async { continuation.resume(returning: decoder.release()) }
            }
            XCTAssertEqual(releaseResult, 0)
        }
    }

    func testNormalAndResolutionHoldModesSendMetadataAndKeepRawObservations() throws {
        for enabled in [false, true] {
            let session = StreamingFrameFeedbackSession(preventResolutionDrops: enabled)
            session.decodeStarted(serverDataKey: 42, atMilliseconds: 100)
            session.decodeCompleted(serverDataKey: 42, atMilliseconds: 132)
            let timing = try XCTUnwrap(session.takeDecodedTiming(serverDataKey: 42, nowMilliseconds: 132))
            session.recordRendered(timing: timing, atMilliseconds: 136)
            let frames = session.pendingFrames(maximumCount: 29, nowMilliseconds: 150)
            let input = IOSStreamInputPacketEncoder.encodeGamepad(sequence: 1, timestampMilliseconds: 150, state: .neutral)
            let packet = try streamInputFrameFeedbackPayload(gamepadPayload: input, frames: frames.map(\.metadata),
                timestampMs: 150, preventResolutionDrops: session.preventResolutionDrops)
            XCTAssertEqual(packet.count, 67)
            XCTAssertEqual(packet[14], 1)
            XCTAssertEqual(Array(packet[27..<31]), [enabled ? 110 : 132, 0, 0, 0])
            XCTAssertEqual(Array(packet[31..<35]), [enabled ? 114 : 136, 0, 0, 0])
            XCTAssertEqual(packet.suffix(24), input.suffix(24))
            session.markSent(frames, wirePayload: packet)
            let snapshot = session.snapshot()
            XCTAssertEqual(snapshot.preventResolutionDrops, enabled)
            XCTAssertEqual(snapshot.lastDecodeDurationMilliseconds, 32)
            XCTAssertEqual(snapshot.lastWireDecodeDurationMilliseconds, enabled ? 10 : 32)
            XCTAssertEqual(snapshot.lastWireDecodedToRenderMilliseconds, 4)
            XCTAssertEqual(snapshot.sentFrameCount, 1)
        }
    }

    func testMetadataRequiresMatchingDecodeAndCompletedRendering() throws {
        let session = StreamingFrameFeedbackSession()
        session.decodeStarted(serverDataKey: UInt32.max, atMilliseconds: 100)
        session.decodeCompleted(serverDataKey: 7, atMilliseconds: 110)
        XCTAssertNil(session.takeDecodedTiming(serverDataKey: 7, nowMilliseconds: 110))
        session.decodeCompleted(serverDataKey: UInt32.max, atMilliseconds: 132)
        let timing = try XCTUnwrap(
            session.takeDecodedTiming(serverDataKey: UInt32.max, nowMilliseconds: 132))
        XCTAssertEqual(session.snapshot().pendingFrameCount, 0)
        session.recordRendered(timing: timing, atMilliseconds: 131)
        XCTAssertEqual(session.snapshot().pendingFrameCount, 0)
        session.recordRendered(timing: timing, atMilliseconds: 136)
        let frames = session.pendingFrames(maximumCount: 29, nowMilliseconds: 150)
        XCTAssertEqual(frames.count, 1)
        XCTAssertEqual(frames[0].metadata.decodedTimeMs, 132)
        let input = IOSStreamInputPacketEncoder.encodeGamepad(
            sequence: UInt32.max, timestampMilliseconds: 150, state: .neutral)
        let packet = try streamInputFrameFeedbackPayload(
            gamepadPayload: input, frames: frames.map(\.metadata), timestampMs: 150,
            preventResolutionDrops: true)
        XCTAssertEqual(packet.count, 67)
        XCTAssertEqual(Array(packet[27..<31]), [110, 0, 0, 0])
        XCTAssertEqual(Array(packet[31..<35]), [114, 0, 0, 0])
        XCTAssertEqual(packet.suffix(24), input.suffix(24))
        XCTAssertEqual(session.snapshot().lastDecodeDurationMilliseconds, 32)
        XCTAssertEqual(session.snapshot().lastDecodedToRenderMilliseconds, 4)
        session.markSent(frames)
        XCTAssertEqual(session.snapshot().pendingFrameCount, 0)
        XCTAssertEqual(session.snapshot().sentFrameCount, 1)
    }

    func testQueuesKeepRecentFramesWithinCapacityAndExpireStaleObservations() throws {
        let session = StreamingFrameFeedbackSession()
        for key in 0..<140 {
            session.decodeStarted(serverDataKey: UInt32(key), atMilliseconds: 1_000 + Double(key))
        }
        XCTAssertEqual(session.snapshot().decodeObservationCount, 128)
        XCTAssertEqual(session.snapshot().discardedDecodeCount, 12)
        for key in 0..<40 {
            let started = 2_000 + Double(key)
            session.decodeStarted(serverDataKey: UInt32(key), atMilliseconds: started)
            session.decodeCompleted(serverDataKey: UInt32(key), atMilliseconds: started + 32)
            let timing = try XCTUnwrap(
                session.takeDecodedTiming(serverDataKey: UInt32(key), nowMilliseconds: started + 32)
            )
            session.recordRendered(timing: timing, atMilliseconds: started + 36)
        }
        XCTAssertEqual(session.snapshot().pendingFrameCount, 32)
        XCTAssertEqual(session.snapshot().discardedFeedbackCount, 8)
        let frames = session.pendingFrames(maximumCount: 100, nowMilliseconds: 2_100)
        XCTAssertEqual(frames.count, 29)
        XCTAssertEqual(frames.first?.timing.serverDataKey, 8)
        XCTAssertTrue(session.pendingFrames(maximumCount: 29, nowMilliseconds: 2_600).isEmpty)
        XCTAssertEqual(session.snapshot().decodeObservationCount, 0)
        XCTAssertEqual(session.snapshot().discardedFeedbackCount, 40)
    }

    func testOldPeerCallbacksAndInvalidClocksStayOutsideNewSession() throws {
        let old = StreamingFrameFeedbackSession()
        old.decodeStarted(serverDataKey: 1, atMilliseconds: 100)
        old.decodeCompleted(serverDataKey: 1, atMilliseconds: 132)
        let timing = try XCTUnwrap(old.takeDecodedTiming(serverDataKey: 1, nowMilliseconds: 132))
        old.invalidate()
        old.recordRendered(timing: timing, atMilliseconds: 136)
        old.decodeStarted(serverDataKey: 2, atMilliseconds: 150)
        XCTAssertFalse(old.snapshot().isActive)
        XCTAssertEqual(old.snapshot().pendingFrameCount, 0)
        XCTAssertEqual(old.snapshot().decodeObservationCount, 0)
        let next = StreamingFrameFeedbackSession()
        for invalid in [Double.nan, .infinity, -1] {
            next.decodeStarted(serverDataKey: 1, atMilliseconds: invalid)
        }
        XCTAssertEqual(next.snapshot().decodeObservationCount, 0)
        XCTAssertTrue(next.pendingFrames(maximumCount: 29, nowMilliseconds: 200).isEmpty)
    }

    func testMetadataWakesIdleInputAndObeysBackpressureAndReleaseGate() {
        var gate = IOSStreamInputSendGate()
        gate.markSent(state: .neutral, at: 100)
        XCTAssertFalse(gate.shouldSend(state: .neutral, bufferedAmount: 0, nowMilliseconds: 110))
        XCTAssertTrue(
            gate.shouldSend(
                state: .neutral, bufferedAmount: 0, nowMilliseconds: 110, hasFrameFeedback: true))
        XCTAssertFalse(
            gate.shouldSend(
                state: .neutral, bufferedAmount: 1_024, nowMilliseconds: 110, hasFrameFeedback: true
            ))
        XCTAssertFalse(
            gate.shouldSend(
                state: .neutral, bufferedAmount: 600, nowMilliseconds: 110, hasFrameFeedback: true))
        XCTAssertTrue(
            gate.shouldSend(
                state: .neutral, bufferedAmount: 512, nowMilliseconds: 110, hasFrameFeedback: true))
        XCTAssertTrue(
            gate.shouldSend(
                state: .neutral, bufferedAmount: 2_048, nowMilliseconds: 111, force: true))
    }

    @MainActor
    func testClarityPreferenceRestoresLegacyAndReachesSessionSnapshot() throws {
        let suite = "XBXRC.ClarityPreference.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        defaults.set(
            Data(#"{"schemaVersion":2,"keepScreenAwake":false}"#.utf8),
            forKey: MobilePreferences.persistenceKey)
        let settings = AppSettingsStore(defaults: defaults)
        XCTAssertFalse(settings.mobile.preventResolutionDrops)
        XCTAssertFalse(settings.mobile.keepScreenAwake)
        settings.mobile.preventResolutionDrops = true
        XCTAssertTrue(settings.streamingSessionSettings.preventResolutionDrops)
        let restored = AppSettingsStore(defaults: defaults)
        XCTAssertTrue(restored.mobile.preventResolutionDrops)
        XCTAssertTrue(restored.streamingSessionSettings.preventResolutionDrops)
        let damaged = try JSONDecoder().decode(
            MobilePreferences.self,
            from: Data(
                #"{"schemaVersion":2,"preventResolutionDrops":"invalid","uiAudio":false}"#.utf8))
        XCTAssertFalse(damaged.preventResolutionDrops)
        XCTAssertFalse(damaged.uiAudio)
    }

    func testWrapperForwardsOriginalDecoderFramesAndLifecycle() throws {
        var pixelBuffer: CVPixelBuffer?
        XCTAssertEqual(
            CVPixelBufferCreate(
                kCFAllocatorDefault, 2, 2, kCVPixelFormatType_32BGRA, nil, &pixelBuffer),
            kCVReturnSuccess)
        let output = RTCVideoFrame(
            buffer: RTCCVPixelBuffer(pixelBuffer: try XCTUnwrap(pixelBuffer)), rotation: ._0,
            timeStampNs: 0)
        let base = FeedbackFixtureDecoder(output: output)
        let session = StreamingFrameFeedbackSession()
        let wrapper = StreamingFeedbackVideoDecoder(base: base, feedback: session)
        var delivered: RTCVideoFrame?
        wrapper.setCallback { delivered = $0 }
        let encoded = RTCEncodedImage()
        encoded.timeStamp = UInt32.max
        XCTAssertEqual(wrapper.startDecode(withNumberOfCores: 4), 0)
        XCTAssertEqual(
            wrapper.decode(encoded, missingFrames: false, codecSpecificInfo: nil, renderTimeMs: 0),
            0)
        XCTAssertTrue(base.encoded === encoded)
        XCTAssertTrue(delivered === output)
        XCTAssertEqual(base.cores, 4)
        XCTAssertNotNil(
            session.takeDecodedTiming(
                serverDataKey: UInt32.max,
                nowMilliseconds: StreamingFrameFeedbackClock.nowMilliseconds))
        XCTAssertEqual(wrapper.implementationName(), "fixtureDecoder")
        XCTAssertEqual(wrapper.release(), 0)
        XCTAssertEqual(base.releaseCount, 1)
    }
}

private final class FeedbackFixtureDecoder: NSObject, RTCVideoDecoder, @unchecked Sendable {
    let output: RTCVideoFrame
    var encoded: RTCEncodedImage?
    var cores: Int32 = 0
    var releaseCount = 0
    private var callback: RTCVideoDecoderCallback?
    init(output: RTCVideoFrame) { self.output = output }
    func setCallback(_ callback: @escaping RTCVideoDecoderCallback) { self.callback = callback }
    func startDecode(withNumberOfCores numberOfCores: Int32) -> Int {
        cores = numberOfCores
        return 0
    }
    func release() -> Int {
        releaseCount += 1
        return 0
    }
    func implementationName() -> String { "fixtureDecoder" }
    func decode(
        _ encodedImage: RTCEncodedImage, missingFrames: Bool,
        codecSpecificInfo info: (any RTCCodecSpecificInfo)?, renderTimeMs: Int64
    ) -> Int {
        encoded = encodedImage
        output.timeStamp = Int32(bitPattern: encodedImage.timeStamp)
        callback?(output)
        return 0
    }
}
