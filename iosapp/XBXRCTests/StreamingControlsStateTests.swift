import CoreGraphics
import SwiftUI
import UIKit
import XCTest

@testable import XBXRC

final class StreamingControlsStateTests: XCTestCase {
    func testTwoFingerInwardGestureRecognizesSymmetricMovement() {
        XCTAssertTrue(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 30, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 130, y: 128),
                    rightEnd: CGPoint(x: 870, y: 124)
                )
            )
        )
    }

    func testTwoFingerInwardGestureRejectsSameDirectionMovement() {
        XCTAssertFalse(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 30, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 130, y: 128),
                    rightEnd: CGPoint(x: 1_000, y: 124)
                )
            )
        )
    }

    func testTwoFingerInwardGestureRejectsVerticalDriftAndLateStart() {
        XCTAssertFalse(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 30, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 130, y: 180),
                    rightEnd: CGPoint(x: 870, y: 124)
                )
            )
        )
        XCTAssertFalse(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 30, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 130, y: 128),
                    rightEnd: CGPoint(x: 870, y: 124),
                    startDeltaMilliseconds: 121
                )
            )
        )
    }

    func testTwoFingerInwardGestureRejectsInvalidStartRegionsAndDuration() {
        XCTAssertFalse(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 260, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 360, y: 128),
                    rightEnd: CGPoint(x: 870, y: 124)
                )
            )
        )
        XCTAssertFalse(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 30, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 130, y: 128),
                    rightEnd: CGPoint(x: 870, y: 124),
                    durationMilliseconds: 451
                )
            )
        )
    }

    func testTwoFingerInwardGestureUsesAdaptiveMinimumTravel() {
        XCTAssertFalse(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 30, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 101, y: 128),
                    rightEnd: CGPoint(x: 899, y: 124)
                )
            )
        )
        XCTAssertTrue(
            StreamingInwardGesturePolicy.recognizes(
                sample(
                    leftStart: CGPoint(x: 30, y: 120),
                    rightStart: CGPoint(x: 970, y: 118),
                    leftEnd: CGPoint(x: 120, y: 128),
                    rightEnd: CGPoint(x: 880, y: 124)
                )
            )
        )
    }

    func testControlsStateReturnsToDockAfterSheet() {
        var state = StreamingControlsState()
        state.reveal()
        XCTAssertEqual(state.surface, .dock)
        state.present(.debugSheet)
        XCTAssertTrue(state.suspendsAutoHide)
        state.dismissPresentedSurface()
        XCTAssertEqual(state.surface, .dock)
        state.hide()
        XCTAssertEqual(state.surface, .hidden)
    }

    func testConnectionSummaryKeepsRawMetricsOutOfUserState() {
        let healthy = diagnostics(rtt: 0.03, jitter: 0.004, frameSupplyDelta: 1)
        XCTAssertEqual(healthy.connectionSummary(featureState: .playing), .good)

        let unstable = diagnostics(rtt: 0.13, jitter: 0.004, frameSupplyDelta: 1)
        XCTAssertEqual(unstable.connectionSummary(featureState: .playing), .unstable)
        XCTAssertEqual(healthy.connectionSummary(featureState: .recovering), .recovering)
    }

    func testMetricsUseDistinctReceiveDecodeSubmitAndPresentCounters() throws {
        var sampler = StreamingMetricsSampler()
        _ = sampler.observe(
            metricDiagnostics(at: 1_000, received: 100, decoded: 90, submitted: 80, presented: 70))
        let next = sampler.observe(
            metricDiagnostics(
                at: 2_000, received: 160, decoded: 140, submitted: 120, presented: 100))
        XCTAssertEqual(next.receivedFps, 60)
        XCTAssertEqual(next.decodedFps, 50)
        XCTAssertEqual(next.submittedFps, 40)
        XCTAssertEqual(next.presentedFps, 30)
        XCTAssertEqual(next.totalBitrateBps, 8_800_000)
        XCTAssertEqual(try XCTUnwrap(next.packetLossPercent), 100.0 / 1001, accuracy: 0.001)
    }

    func testMetricsFirstDuplicateStaleAndCounterResetKeepMissingValues() {
        var sampler = StreamingMetricsSampler()
        let first = metricDiagnostics(
            at: 1_000, received: 100, decoded: 90, submitted: 80, presented: 70)
        XCTAssertNil(sampler.observe(first).presentedFps)
        let next = metricDiagnostics(
            at: 2_000, received: 160, decoded: 140, submitted: 120, presented: 100)
        let metrics = sampler.observe(next)
        XCTAssertEqual(sampler.observe(next), metrics)
        var stale = next
        stale.metrics = metrics
        let model = StreamingDiagnosticsModel(
            diagnostics: stale, state: .playing, picture: .init(), nowMilliseconds: 6_000)
        XCTAssertEqual(model.observation, "stale")
        XCTAssertEqual(model.presentFps, "--")
        XCTAssertEqual(model.videoBitrate, "--")
        XCTAssertEqual(model.packetLoss, "--")
        XCTAssertNil(
            sampler.observe(
                metricDiagnostics(at: 2_500, received: 1, decoded: 1, submitted: 1, presented: 1)
            ).receivedFps)
    }

    func testMetricZeroProgressShowsZeroAndPartialStatsPreserveUnknown() {
        var sampler = StreamingMetricsSampler()
        let first = metricDiagnostics(
            at: 1_000, received: 100, decoded: 90, submitted: 80, presented: 70)
        _ = sampler.observe(first)
        var next = metricDiagnostics(
            at: 2_000, received: 100, decoded: 90, submitted: 80, presented: 70)
        next.metrics = sampler.observe(next)
        let model = StreamingDiagnosticsModel(
            diagnostics: next, state: .playing, picture: .init(), nowMilliseconds: 2_000)
        XCTAssertEqual(model.presentFps, "0")
        XCTAssertEqual(model.notice, "画面暂时停滞")
        var partial = metricDiagnostics(
            at: 3_000, received: 160, decoded: 140, submitted: 120, presented: 100)
        partial = StreamingSessionDiagnostics(
            peerStats: nil, displayHealth: partial.displayHealth, peer: nil,
            audioLifecycleState: nil, audioRoute: nil, audioTrackReady: false,
            updatedAtMilliseconds: 3_000)
        XCTAssertNil(sampler.observe(partial).receivedFps)
        XCTAssertEqual(
            StreamingDiagnosticsModel(
                diagnostics: partial, state: .playing, picture: .init(), nowMilliseconds: 3_000
            ).rtt, "--")
    }

    func testDiagnosticClipboardUsesFourGroupsAndExcludesRoutes() throws {
        var diagnostics = metricDiagnostics(
            at: 2_000, received: 160, decoded: 140, submitted: 120, presented: 100)
        diagnostics.metrics = .init(sampledAtMilliseconds: 2_000)
        let model = StreamingDiagnosticsModel(
            diagnostics: diagnostics, state: .playing, picture: .init(), nowMilliseconds: 2_000)
        let data = try XCTUnwrap(model.clipboardSummary.data(using: .utf8))
        let json = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data) as? [String: [String: String]])
        XCTAssertEqual(Set(json.keys), Set(["network", "display", "recovery", "session"]))
        XCTAssertNil(json["session"]?["transportPath"])
        XCTAssertNil(json["session"]?["audioRoute"])
        XCTAssertEqual(json["display"]?["sourceSize"], "1920 × 1080")
        XCTAssertEqual(model.rows(.display).first { $0.id == "srRuntime" }?.value, "标准缩放")
    }

    @MainActor
    func testStandardTouchLayoutSharesCatalogAndReleasesMultiPointerState() throws {
        let layout = StreamingTouchLayoutController()
        XCTAssertNil(layout.loadError)
        XCTAssertEqual(layout.input.layout, try TouchLayoutCatalog.standard())
        let session = StreamInputSession(controller: { nil })
        _ = session.outputState(now: 0)
        layout.bind(to: session)
        layout.input.setViewport(size: CGSize(width: 800, height: 420))
        let a = try XCTUnwrap(
            layout.input.sockets.first { $0.control.action == .gamepad("gamepadA") })
        let trigger = try XCTUnwrap(
            layout.input.sockets.first { $0.control.action == .gamepad("leftTrigger") })
        layout.input.process([
            .init(id: 1, phase: .began, position: a.center),
            .init(id: 2, phase: .began, position: trigger.center),
        ])
        let pressed = session.outputState(now: 1)
        XCTAssertEqual(pressed.buttonMask, 16)
        XCTAssertEqual(pressed.leftTrigger, 1)
        layout.input.process([.init(id: 1, phase: .ended, position: a.center)])
        XCTAssertEqual(session.outputState(now: 2).buttonMask, 0)
        XCTAssertEqual(session.outputState(now: 2).leftTrigger, 1)
        session.setSuppressed(true)
        layout.input.cancelAll(reason: "inputSuppressed")
        XCTAssertEqual(session.outputState(now: 3), .neutral)
        session.setSuppressed(false)
        XCTAssertEqual(session.outputState(now: 4), .neutral)
        layout.unbind(from: session)
        XCTAssertEqual(layout.input.gamepad, .neutral)
        XCTAssertEqual(session.outputState(now: 5), .neutral)
    }

    @MainActor
    func testStreamingTouchSurfacePassesEmptySpaceAndKeepsControlHitAreas() throws {
        let input = TouchInputAccumulator()
        input.load(try TouchLayoutCatalog.standard())
        let surface = TouchLayoutSurfaceView(input: input)
        surface.capturesEmptySpace = false
        for size in [
            CGSize(width: 390, height: 700), CGSize(width: 800, height: 340),
            CGSize(width: 1194, height: 760),
        ] {
            surface.frame = CGRect(origin: .zero, size: size)
            surface.layoutSubviews()
            let center = CGPoint(x: size.width / 2, y: size.height / 2)
            XCTAssertFalse(surface.point(inside: center, with: nil))
            for socket in input.sockets {
                XCTAssertTrue(surface.point(inside: socket.center, with: nil), socket.id)
                XCTAssertTrue(surface.bounds.contains(socket.frame), "\(size): \(socket.id)")
            }
        }
        surface.stop()
    }

    @MainActor
    func testPortraitStreamingViewportPlacesAllGamepadControlsInLowerHalf() throws {
        let layout = try TouchLayoutCatalog.standard()
        let content = TouchLayoutResolver.resolve(layout, activeLayers: [])
        for size in [
            CGSize(width: 390, height: 844), CGSize(width: 768, height: 1024),
            CGSize(width: 844, height: 390),
        ] {
            let frame = StreamingTouchViewportPolicy.frame(in: size)
            for socket in TouchLayoutGeometry.sockets(content: content, size: frame.size) {
                let actual = socket.frame.offsetBy(dx: frame.minX, dy: frame.minY)
                XCTAssertTrue(CGRect(origin: .zero, size: size).contains(actual), socket.id)
                if size.height > size.width {
                    XCTAssertGreaterThanOrEqual(actual.minY, size.height / 2, socket.id)
                }
            }
        }
    }

    func testDiagnosticsPlacementPreservesVisibleGameAreaAcrossOrientations() {
        for available in [
            CGSize(width: 390, height: 740), CGSize(width: 844, height: 350),
            CGSize(width: 1194, height: 800),
        ] {
            let panel = StreamingDiagnosticsPlacement.size(in: available)
            XCTAssertLessThan(panel.width * panel.height, available.width * available.height * 0.6)
            XCTAssertLessThan(panel.width, available.width)
            XCTAssertLessThan(panel.height, available.height)
        }
    }

    @MainActor
    func testXboxGuideDrawsStandardAssetAndKeepsSymbolCached() throws {
        XCTAssertNotNil(UIImage(named: "XboxLogo"))
        let input = TouchInputAccumulator()
        input.load(try TouchLayoutCatalog.standard())
        let surface = TouchLayoutSurfaceView(input: input)
        surface.frame = CGRect(x: 0, y: 0, width: 800, height: 420)
        surface.layoutSubviews()
        let socket = try XCTUnwrap(input.sockets.first { $0.control.action == .gamepad("guide") })
        let layer = try XCTUnwrap(surface.layer.sublayers?.first { $0.position == socket.center })
        let symbol = try XCTUnwrap(layer.sublayers?[3])
        let artwork = try XCTUnwrap(symbol.contents) as AnyObject
        XCTAssertEqual(CFGetTypeID(artwork), CGImage.typeID)
        XCTAssertTrue(try XCTUnwrap(layer.sublayers?[1]).isHidden)
        surface.requestRender()
        XCTAssertTrue(symbol.contents as AnyObject? === artwork)
        surface.stop()
    }

    @MainActor
    func testStreamingPanelsRenderPhoneTabletAndEnglishLayouts() async throws {
        var sampler = StreamingMetricsSampler()
        _ = sampler.observe(
            metricDiagnostics(at: 1_000, received: 100, decoded: 90, submitted: 80, presented: 70))
        var diagnostics = metricDiagnostics(
            at: 2_000, received: 160, decoded: 140, submitted: 120, presented: 100)
        diagnostics.metrics = sampler.observe(diagnostics)
        let model = StreamingDiagnosticsModel(
            diagnostics: diagnostics, state: .playing, picture: .init(), nowMilliseconds: 2_000)
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "XBXRC.StreamingPanels.Tests"))
        defer { defaults.removePersistentDomain(forName: "XBXRC.StreamingPanels.Tests") }
        let settings = AppSettingsStore(defaults: defaults)
        for (name, size, locale) in [
            ("phone", CGSize(width: 390, height: 844), "zh-Hans"),
            ("phone-landscape", CGSize(width: 844, height: 390), "zh-Hans"),
            ("ipad", CGSize(width: 1194, height: 834), "en"),
        ] {
            let session = StreamInputSession(controller: { nil })
            try await renderPanel(
                ZStack(alignment: .top) {
                    Color.black.ignoresSafeArea()
                    TouchGamepadOverlay(input: session)
                    StreamingRuntimeMetricsBar(model: model).padding(.horizontal, 64).padding(
                        .top, 8)
                }.environmentObject(settings).environment(\.locale, Locale(identifier: locale)),
                size: size, name: "\(name)-streaming-standard-touch")
        }
        for group in StreamingDiagnosticGroup.allCases {
            try await renderPanel(
                StreamingDiagnosticsPanel(
                    model: model, group: .constant(group), copied: .constant(false)
                )
                .environmentObject(settings).appThemeCanvas().environment(
                    \.locale, Locale(identifier: "zh-Hans")),
                size: CGSize(width: 390, height: 700), name: "phone-diagnostics-\(group.rawValue)")
        }
    }

    @MainActor
    private func renderPanel<Content: View>(_ content: Content, size: CGSize, name: String)
        async throws
    {
        let scene = try XCTUnwrap(
            UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let previous = scene.windows.first { $0.isKeyWindow }
        let window = UIWindow(windowScene: scene)
        let host = UIHostingController(rootView: content)
        window.frame = CGRect(origin: .zero, size: size)
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer {
            window.isHidden = true
            window.rootViewController = nil
            previous?.makeKey()
        }
        host.view.frame = CGRect(origin: .zero, size: size)
        try await Task.sleep(for: .milliseconds(250))
        host.view.layoutIfNeeded()
        let image = UIGraphicsImageRenderer(size: size).image { _ in
            host.view.drawHierarchy(in: CGRect(origin: .zero, size: size), afterScreenUpdates: true)
        }
        XCTAssertEqual(image.size, size)
        let attachment = XCTAttachment(image: image)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func metricDiagnostics(
        at: Double, received: UInt64, decoded: UInt64, submitted: Int64, presented: Int64
    ) -> StreamingSessionDiagnostics {
        var stats = StreamingPeerStats(
            inboundVideoBytes: UInt64(at) * 1_000, inboundAudioBytes: UInt64(at) * 100,
            framesDecoded: decoded, framesDropped: 0, packetsLost: Int64(at / 1_000),
            roundTripTimeSeconds: 0.03, jitterSeconds: 0.004, packetsReceived: UInt64(at),
            receiveBitrateBps: 8_000_000, frameSupplyDelta: 50)
        stats.observedAtMilliseconds = at
        stats.framesReceived = received
        stats.observedFramesDecoded = decoded
        stats.videoPacketsLost = Int64(at / 1_000)
        var display = StreamingDisplayHealthSnapshot.zero
        display = StreamingDisplayHealthSnapshot(
            submitCount: submitted, presentCount: presented, lastSubmitAtMilliseconds: at - 10,
            lastPresentAtMilliseconds: at - 5, submitToPresentP50Milliseconds: 4,
            submitToPresentP95Milliseconds: 8, displayedFrameDelta: 30, mailboxOverwriteCount: 2,
            dropCountByReason: [:], currentDrawableBusy: false,
            submitToPresentP99Milliseconds: 12,
            configuration: .init(
                sourceWidth: 1920, sourceHeight: 1080, outputWidth: 2532, outputHeight: 1424,
                superResolutionActive: false, sharpness: 0))
        return StreamingSessionDiagnostics(
            peerStats: stats, displayHealth: display, peer: nil, audioLifecycleState: "active",
            audioRoute: "private-device-route", audioTrackReady: true, updatedAtMilliseconds: at,
            connectedElapsedMilliseconds: 900, mediaReadyElapsedMilliseconds: 1_200)
    }

    private func sample(
        leftStart: CGPoint,
        rightStart: CGPoint,
        leftEnd: CGPoint,
        rightEnd: CGPoint,
        startDeltaMilliseconds: Double = 20,
        durationMilliseconds: Double = 260
    ) -> StreamingInwardGestureSample {
        StreamingInwardGestureSample(
            leftStart: leftStart,
            rightStart: rightStart,
            leftEnd: leftEnd,
            rightEnd: rightEnd,
            startDeltaMilliseconds: startDeltaMilliseconds,
            durationMilliseconds: durationMilliseconds,
            viewWidth: 1_000
        )
    }

    private func diagnostics(
        rtt: Double,
        jitter: Double,
        frameSupplyDelta: Int64
    ) -> StreamingSessionDiagnostics {
        StreamingSessionDiagnostics(
            peerStats: StreamingPeerStats(
                inboundVideoBytes: 1,
                inboundAudioBytes: 1,
                framesDecoded: 1,
                framesDropped: 0,
                packetsLost: 0,
                roundTripTimeSeconds: rtt,
                jitterSeconds: jitter,
                packetsReceived: 1,
                frameSupplyDelta: frameSupplyDelta
            ),
            displayHealth: nil,
            peer: nil,
            audioLifecycleState: nil,
            audioRoute: nil,
            audioTrackReady: true,
            updatedAtMilliseconds: 1
        )
    }
}
