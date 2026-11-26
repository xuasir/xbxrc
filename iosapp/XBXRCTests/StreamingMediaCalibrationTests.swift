import AVFoundation
import CoreVideo
import Foundation
@preconcurrency import WebRTC
import XCTest
import simd

@testable import XBXRC

final class StreamingMediaCalibrationTests: XCTestCase {
    func testTimingUsesWindowDeltasForBufferTargetMinimumAndDecode() throws {
        var tracker = StreamingMediaTimingTracker()
        let first = StreamingMediaTimingCounters(
            jitterBufferDelaySeconds: 10, jitterBufferEmittedCount: 1000,
            jitterBufferTargetDelaySeconds: 12, jitterBufferMinimumDelaySeconds: 6,
            totalDecodeTimeSeconds: 2, framesDecoded: 1000)
        let second = StreamingMediaTimingCounters(
            jitterBufferDelaySeconds: 10.3, jitterBufferEmittedCount: 1060,
            jitterBufferTargetDelaySeconds: 12.6, jitterBufferMinimumDelaySeconds: 6.18,
            totalDecodeTimeSeconds: 2.06, framesDecoded: 1060)
        XCTAssertNil(
            tracker.observe(streams: ["video": first], atMilliseconds: 1000).decodeMilliseconds)
        let sample = tracker.observe(streams: ["video": second], atMilliseconds: 2000)
        XCTAssertEqual(try XCTUnwrap(sample.jitterBufferMilliseconds), 5, accuracy: 0.0001)
        XCTAssertEqual(try XCTUnwrap(sample.jitterBufferTargetMilliseconds), 10, accuracy: 0.0001)
        XCTAssertEqual(try XCTUnwrap(sample.jitterBufferMinimumMilliseconds), 3, accuracy: 0.0001)
        XCTAssertEqual(try XCTUnwrap(sample.decodeMilliseconds), 1, accuracy: 0.0001)
        XCTAssertEqual(sample.windowMilliseconds, 1000)
        XCTAssertEqual(sample.totals.jitterBufferEmittedCount, 1060)
    }

    func testTimingResetsEachStreamAndWeightsByEmittedFrames() throws {
        var tracker = StreamingMediaTimingTracker()
        let empty = StreamingMediaTimingCounters(
            jitterBufferDelaySeconds: 0, jitterBufferEmittedCount: 0)
        _ = tracker.observe(streams: ["a": empty, "b": empty], atMilliseconds: 1000)
        let a = StreamingMediaTimingCounters(
            jitterBufferDelaySeconds: 0.02, jitterBufferEmittedCount: 10)
        let b = StreamingMediaTimingCounters(
            jitterBufferDelaySeconds: 0.16, jitterBufferEmittedCount: 20)
        let weighted = tracker.observe(streams: ["a": a, "b": b], atMilliseconds: 2000)
        XCTAssertEqual(try XCTUnwrap(weighted.jitterBufferMilliseconds), 6, accuracy: 0.0001)
        XCTAssertNil(
            tracker.observe(streams: ["replacement": b], atMilliseconds: 3000)
                .jitterBufferMilliseconds)
        XCTAssertNil(
            tracker.observe(streams: ["replacement": empty], atMilliseconds: 4000)
                .jitterBufferMilliseconds)
        let resumed = tracker.observe(streams: ["replacement": a], atMilliseconds: 5000)
        XCTAssertEqual(try XCTUnwrap(resumed.jitterBufferMilliseconds), 2, accuracy: 0.0001)
    }

    func testTimingKeepsMissingSilentInvalidAndInterruptedIntervalsUnknown() {
        var tracker = StreamingMediaTimingTracker()
        let empty = StreamingMediaTimingCounters(
            jitterBufferDelaySeconds: 0, jitterBufferEmittedCount: 0)
        _ = tracker.observe(streams: ["video": empty], atMilliseconds: 1000)
        XCTAssertNil(
            tracker.observe(streams: ["video": empty], atMilliseconds: 2000)
                .jitterBufferMilliseconds)
        let invalid = StreamingMediaTimingCounters(
            jitterBufferDelaySeconds: .nan, jitterBufferEmittedCount: 60)
        let sample = tracker.observe(streams: ["video": invalid], atMilliseconds: 3000)
        XCTAssertNil(sample.totals.jitterBufferDelaySeconds)
        XCTAssertNil(sample.jitterBufferMilliseconds)
        XCTAssertEqual(sample.tracePayload()["jitterBufferTargetMs"], .null)
        XCTAssertNil(
            tracker.observe(streams: ["video": empty], atMilliseconds: 5000).windowMilliseconds)
        XCTAssertNil(tracker.observe(streams: [:], atMilliseconds: 6000).totals.framesDecoded)
    }

    func testYUVRangePreservesExactBlackAndWhiteForAllMatrices() {
        for matrix in [
            kCVImageBufferYCbCrMatrix_ITU_R_601_4,
            kCVImageBufferYCbCrMatrix_ITU_R_709_2, kCVImageBufferYCbCrMatrix_ITU_R_2020,
        ] {
            for fullRange in [true, false] {
                let color = StreamingVideoColorConfiguration(
                    width: 1920, height: 1080, fullRange: fullRange,
                    matrixAttachment: matrix as String)
                let conversion = color.conversion
                let levels: [(Float, Float)] = [
                    (fullRange ? 0 : 16.0 / 255.0, 0), (fullRange ? 1 : 235.0 / 255.0, 1),
                ]
                for (luma, expected) in levels {
                    let rgb =
                        conversion.matrix
                        * (SIMD3<Float>(luma, 128.0 / 255.0, 128.0 / 255.0) + conversion.offset)
                    for component in [rgb.x, rgb.y, rgb.z] {
                        XCTAssertEqual(component, expected, accuracy: 0.00001)
                    }
                }
            }
        }
    }

    func testBT709AndBT2020RedUseSourceCoefficients() {
        let sources: [(CFString, Float, Float)] = [
            (kCVImageBufferYCbCrMatrix_ITU_R_709_2, 0.2126, 0.0722),
            (kCVImageBufferYCbCrMatrix_ITU_R_2020, 0.2627, 0.0593),
        ]
        for (attachment, red, blue) in sources {
            let color = StreamingVideoColorConfiguration(
                width: 1920, height: 1080, fullRange: true, matrixAttachment: attachment as String)
            let cb = -red / (2 * (1 - blue))
            let yuv = SIMD3<Float>(red, cb + 128.0 / 255.0, 0.5 + 128.0 / 255.0)
            let conversion = color.conversion
            let rgb = conversion.matrix * (yuv + conversion.offset)
            XCTAssertEqual(rgb.x, 1, accuracy: 0.00001)
            XCTAssertEqual(rgb.y, 0, accuracy: 0.00001)
            XCTAssertEqual(rgb.z, 0, accuracy: 0.00001)
        }
    }

    func testPixelBufferMetadataOverridesResolutionAndSurvivesMailbox() throws {
        var buffer: CVPixelBuffer?
        XCTAssertEqual(
            CVPixelBufferCreate(
                kCFAllocatorDefault, 1280, 720,
                kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, nil, &buffer), kCVReturnSuccess)
        let pixelBuffer = try XCTUnwrap(buffer)
        CVBufferSetAttachment(
            pixelBuffer, kCVImageBufferYCbCrMatrixKey,
            kCVImageBufferYCbCrMatrix_ITU_R_601_4, .shouldPropagate)
        CVBufferSetAttachment(
            pixelBuffer, kCVImageBufferColorPrimariesKey,
            kCVImageBufferColorPrimaries_ITU_R_709_2, .shouldPropagate)
        CVBufferSetAttachment(
            pixelBuffer, kCVImageBufferTransferFunctionKey,
            kCVImageBufferTransferFunction_sRGB, .shouldPropagate)
        let mailbox = StreamingLatestFrameMailbox()
        let frame = mailbox.makeFrame(
            storage: .pixelBuffer(pixelBuffer), width: 1280, height: 720,
            rotationDegrees: 0, isFullRange: false, traceContext: nil)
        XCTAssertEqual(frame.colorConfiguration.matrix, .bt601)
        XCTAssertEqual(frame.colorConfiguration.matrixSource, "pixelBufferAttachment")
        XCTAssertEqual(
            frame.colorConfiguration.primaries, kCVImageBufferColorPrimaries_ITU_R_709_2 as String)
        XCTAssertEqual(
            frame.colorConfiguration.transferFunction, kCVImageBufferTransferFunction_sRGB as String
        )
        XCTAssertNotNil(frame.colorConfiguration.makeOutputColorSpace())
        XCTAssertEqual(
            StreamingVideoColorConfiguration(width: 1920, height: 1080, fullRange: false).matrix,
            .bt709)
        XCTAssertEqual(
            StreamingVideoColorConfiguration(width: 720, height: 480, fullRange: false).matrix,
            .bt601)
    }

    func testAudioPreferencesKeepDefaultADMFormatAcrossStereoAndRoutes() {
        let defaults = RTCAudioSessionConfiguration()
        XCTAssertEqual(defaults.inputNumberOfChannels, 1)
        XCTAssertEqual(defaults.outputNumberOfChannels, 1)
        XCTAssertEqual(defaults.ioBufferDuration, 0.020, accuracy: 0.000001)
        let speaker = StreamingAudioOutputPreferences(
            stereoAudio: true, maximumChannels: 2,
            outputPortTypes: [AVAudioSession.Port.builtInSpeaker.rawValue])
        XCTAssertEqual(speaker.negotiatedChannels, 2)
        XCTAssertEqual(speaker.preferredChannels, defaults.outputNumberOfChannels)
        XCTAssertEqual(speaker.sampleRate, defaults.sampleRate)
        XCTAssertEqual(speaker.ioBufferDuration, defaults.ioBufferDuration)
        XCTAssertEqual(
            StreamingAudioOutputPreferences(stereoAudio: true, maximumChannels: 1)
                .preferredChannels, 1)
        XCTAssertEqual(
            StreamingAudioOutputPreferences(stereoAudio: false, maximumChannels: 2)
                .preferredChannels, 1)
        XCTAssertEqual(
            StreamingAudioOutputPreferences(
                stereoAudio: true, negotiatedChannels: 1, maximumChannels: 2
            ).preferredChannels, 1)
        let hfp = StreamingAudioOutputPreferences(
            stereoAudio: true, maximumChannels: 2,
            outputPortTypes: [AVAudioSession.Port.bluetoothHFP.rawValue])
        let a2dp = StreamingAudioOutputPreferences(
            stereoAudio: true, maximumChannels: 2,
            outputPortTypes: [AVAudioSession.Port.bluetoothA2DP.rawValue])
        XCTAssertEqual(hfp.preferredChannels, 1)
        XCTAssertEqual(hfp.ioBufferDuration, defaults.ioBufferDuration)
        XCTAssertEqual(a2dp.preferredChannels, defaults.outputNumberOfChannels)
        XCTAssertEqual(a2dp.ioBufferDuration, defaults.ioBufferDuration)
    }

    func testAudioConfigurationsKeepDefaultPCMAndBufferWithStereoNegotiation() {
        let defaults = RTCAudioSessionConfiguration()
        let listen = StreamingAudioLifecycleCoordinator.makeConfiguration(
            for: .receiveOnly, stereoAudio: true, maximumOutputChannels: 2)
        let voice = StreamingAudioLifecycleCoordinator.makeConfiguration(
            for: .sendReceive, stereoAudio: true, negotiatedOutputChannels: 1,
            maximumOutputChannels: 2)
        XCTAssertEqual(listen.outputNumberOfChannels, defaults.outputNumberOfChannels)
        XCTAssertEqual(voice.outputNumberOfChannels, 1)
        XCTAssertEqual(listen.inputNumberOfChannels, 1)
        XCTAssertEqual(listen.mode, AVAudioSession.Mode.voiceChat.rawValue)
        XCTAssertEqual(listen.sampleRate, defaults.sampleRate)
        XCTAssertEqual(listen.ioBufferDuration, defaults.ioBufferDuration)
        XCTAssertEqual(voice.sampleRate, defaults.sampleRate)
        XCTAssertEqual(voice.ioBufferDuration, defaults.ioBufferDuration)
        XCTAssertTrue(listen.categoryOptions.contains(.allowBluetoothHFP))
        XCTAssertTrue(voice.categoryOptions.contains(.allowBluetoothHFP))
    }

    func testNativeNegotiationAndHealthyRouteOnlyUpdateObservations() async throws {
        let recorder = StreamingAudioRegressionEventRecorder()
        let routeObserved = expectation(description: "健康路由只更新音频观测")
        let coordinator = StreamingAudioLifecycleCoordinator { event, payload in
            recorder.record(event, payload: payload)
            if event == "audioRouteChanged",
                payload["routeChangeReason"] == .string("wakeFromSleep")
            {
                routeObserved.fulfill()
            }
        }
        try coordinator.start(audioDirection: .receiveOnly, stereoAudio: true)
        defer { coordinator.stop() }
        let session = RTCAudioSession.sharedInstance()
        let initial = RTCAudioSessionConfiguration.webRTC()
        XCTAssertEqual(initial.outputNumberOfChannels, 1)
        XCTAssertEqual(initial.inputNumberOfChannels, 1)
        XCTAssertEqual(initial.ioBufferDuration, 0.020, accuracy: 0.000001)
        XCTAssertFalse(session.currentRoute.outputs.isEmpty)
        let activationCount = recorder.count("audioSessionActivationStarted")
        for channels in [2, 1, 2] {
            coordinator.updateNegotiatedOutputChannels(channels)
            XCTAssertTrue(RTCAudioSessionConfiguration.webRTC() === initial)
            let payload = try XCTUnwrap(recorder.latest("audioOutputObserved"))
            XCTAssertEqual(payload["negotiatedOutputChannels"], .integer(Int64(channels)))
            XCTAssertEqual(payload["nativePlayoutChannels"], .integer(1))
            XCTAssertEqual(payload["requestedIOBufferDurationMs"], .double(20))
            XCTAssertTrue(session.isAudioEnabled)
        }
        coordinator.audioSessionDidChangeRoute(
            session, reason: .wakeFromSleep, previousRoute: session.currentRoute)
        await fulfillment(of: [routeObserved], timeout: 2)
        // sync 观测同时作为队列屏障，确保路由事件及后续观测处理完成。
        coordinator.updateNegotiatedOutputChannels(2)
        XCTAssertTrue(RTCAudioSessionConfiguration.webRTC() === initial)
        XCTAssertEqual(recorder.count("audioSessionActivationStarted"), activationCount)
        XCTAssertTrue(session.isAudioEnabled)
    }

    func testAudioNegotiationUsesSenderStereoAndScopesPayloadToAudio() {
        let sdp = """
            v=0
            m=video 9 RTP/SAVPF 111
            a=rtpmap:111 H264/90000
            m=audio 9 RTP/SAVPF 111
            a=rtpmap:111 opus/48000/2
            a=fmtp:111 stereo=0;sprop-stereo=1
            """
        XCTAssertEqual(
            StreamingAudioOutputPreferences.negotiatedChannels(in: sdp, stereoRequested: true), 2)
        XCTAssertEqual(
            StreamingAudioOutputPreferences.negotiatedChannels(in: sdp, stereoRequested: false), 1)
        XCTAssertEqual(
            StreamingAudioOutputPreferences.negotiatedChannels(
                in: sdp.replacingOccurrences(of: "sprop-stereo=1", with: "sprop-stereo=0"),
                stereoRequested: true), 1)
        XCTAssertNil(
            StreamingAudioOutputPreferences.negotiatedChannels(
                in: "m=audio 0 RTP/SAVPF 111\na=rtpmap:111 opus/48000/2", stereoRequested: true))
    }
}

private final class StreamingAudioRegressionEventRecorder: @unchecked Sendable {
    private let lock = NSLock()
    private var events: [(String, [String: IOSRuntimeTraceValue])] = []

    func record(_ event: String, payload: [String: IOSRuntimeTraceValue]) {
        lock.lock()
        defer { lock.unlock() }
        events.append((event, payload))
    }

    func count(_ event: String) -> Int {
        lock.lock()
        defer { lock.unlock() }
        return events.filter { $0.0 == event }.count
    }

    func latest(_ event: String) -> [String: IOSRuntimeTraceValue]? {
        lock.lock()
        defer { lock.unlock() }
        return events.last { $0.0 == event }?.1
    }
}
