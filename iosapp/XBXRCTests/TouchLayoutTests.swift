import UIKit
import XCTest

@testable import XBXRC

@MainActor
final class TouchLayoutTests: XCTestCase {
    private func engine(_ name: String = "generic", index: Int = 0) throws -> TouchInputAccumulator
    {
        let input = TouchInputAccumulator()
        input.setViewport(size: CGSize(width: 800, height: 420))
        input.load(try TouchLayoutCatalog.read(name)[index])
        return input
    }

    private func socket(_ input: TouchInputAccumulator, action: TouchAction) throws
        -> TouchControlSocket
    {
        try XCTUnwrap(input.sockets.first { $0.control.action == action })
    }

    private func sample(_ id: Int, _ phase: TouchPointerPhase, _ point: CGPoint)
        -> TouchPointerSample
    {
        .init(id: id, phase: phase, position: point)
    }

    func testRealLayoutsAndCapabilityReports() throws {
        XCTAssertEqual(try TouchLayoutCatalog.read("forza-layouts").count, 2)
        for choice in try TouchLayoutCatalog.builtIn() {
            XCTAssertTrue(
                choice.layout.report.isSupported, "\(choice.id): \(choice.layout.report.issues)")
        }
        let halo = try TouchLayoutCatalog.read("halo-layouts")
        XCTAssertFalse(halo.isEmpty)
        XCTAssertTrue(halo.contains { !$0.report.isSupported })
        XCTAssertTrue(halo.flatMap { $0.report.issues }.contains { $0.path.contains("axis") })
    }

    func testClusterNullInheritanceAndBlankReplacement() throws {
        let button = TouchJSON.object(["type": .string("button"), "action": .string("gamepadA")])
        let cluster = TouchJSON.array([button, .null])
        XCTAssertEqual(
            TouchLayoutResolver.mergeSlots([cluster, button], [.null, .null]), [cluster, button])
        XCTAssertEqual(
            TouchLayoutResolver.mergeSlots([button], [.object(["type": .string("blank")])]),
            [.object(["type": .string("blank")])])
        let input = try engine("forza-layouts")
        XCTAssertNotNil(input.sockets.first { $0.id == "left/outer/2/1" })
        XCTAssertNotNil(input.sockets.first { $0.id == "left/outer/4/0" })
        XCTAssertNil(input.sockets.first { $0.id == "left/outer/1" })
    }

    func testEveryVisibleControlCenterHitsItsControlAcrossSizes() throws {
        let input = try engine()
        for choice in try TouchLayoutCatalog.builtIn() {
            input.load(choice.layout)
            for size in [
                CGSize(width: 393, height: 610), CGSize(width: 734, height: 215),
                CGSize(width: 1024, height: 1120), CGSize(width: 1280, height: 800),
            ] {
                input.setViewport(size: size)
                for socket in input.sockets {
                    XCTAssertEqual(
                        TouchLayoutGeometry.hitTest(socket.center, sockets: input.sockets)?.id,
                        socket.id, "\(choice.id) \(size) \(socket.id)")
                    XCTAssertTrue(
                        CGRect(origin: .zero, size: size).contains(socket.frame),
                        "\(choice.id) \(size) \(socket.id) \(socket.frame)")
                }
            }
        }
    }

    func testTwoPointersSharingOneButtonReleaseIndependently() throws {
        let input = try engine()
        let button = try socket(input, action: .gamepad("gamepadA"))
        input.process([sample(1, .began, button.center), sample(2, .began, button.center)])
        XCTAssertEqual(input.gamepad.buttonMask, 16)
        input.process([sample(1, .ended, button.center)])
        XCTAssertEqual(input.gamepad.buttonMask, 16)
        input.process([sample(2, .ended, button.center)])
        XCTAssertEqual(input.gamepad, .neutral)
    }

    func testDifferentControlsSharingOneActionReleaseIndependently() throws {
        let input = try engine("forza-layouts")
        let gas = try XCTUnwrap(input.sockets.first { $0.id == "right/inner/0" })
        let handbrake = try socket(input, action: .gamepad("gamepadA"))
        let pull = CGPoint(x: gas.center.x, y: gas.center.y - gas.unitSize)
        input.process([
            sample(1, .began, gas.center), sample(1, .moved, pull),
            sample(2, .began, handbrake.center),
        ])
        XCTAssertEqual(input.gamepad.buttonMask & 16, 16)
        XCTAssertEqual(input.gamepad.rightTrigger, 1)
        input.process([sample(2, .ended, handbrake.center)])
        XCTAssertEqual(input.gamepad.buttonMask & 16, 16)
        input.process([sample(1, .ended, pull)])
        XCTAssertEqual(input.gamepad, .neutral)
    }

    func testShortTapKeepsBothEdgesWithinOneBatch() throws {
        let input = try engine()
        let button = try socket(input, action: .gamepad("gamepadA"))
        var frame: TouchInputFrame?
        input.onInput = { frame = $0 }
        input.process([sample(1, .began, button.center), sample(1, .ended, button.center)])
        XCTAssertEqual(frame?.events.map(\.phase), [.began, .ended])
        XCTAssertEqual(frame?.transitions.map(\.buttonMask), [16, 0])
        XCTAssertEqual(frame?.gamepad, .neutral)
    }

    func testSimultaneousStickButtonAndTrigger() throws {
        let input = try engine()
        let stick = try XCTUnwrap(input.sockets.first { $0.id == "left/inner/0" })
        let a = try socket(input, action: .gamepad("gamepadA"))
        let trigger = try socket(input, action: .gamepad("rightTrigger"))
        input.process([
            sample(1, .began, stick.center), sample(2, .began, a.center),
            sample(3, .began, trigger.center),
            sample(1, .moved, CGPoint(x: stick.center.x + 100, y: stick.center.y - 100)),
        ])
        XCTAssertEqual(input.gamepad.buttonMask & 16, 16)
        XCTAssertEqual(input.gamepad.rightTrigger, 1)
        XCTAssertEqual(input.gamepad.leftThumbX, 0.7071, accuracy: 0.001)
        XCTAssertEqual(input.gamepad.leftThumbY, 0.7071, accuracy: 0.001)
        input.process([sample(2, .ended, a.center)])
        XCTAssertGreaterThan(input.gamepad.leftThumbX, 0)
        XCTAssertEqual(input.gamepad.rightTrigger, 1)
        input.cancelAll()
        XCTAssertEqual(input.gamepad, .neutral)
    }

    func testOrdinaryButtonRollsOverAndReleasesOutside() throws {
        let input = try engine()
        let a = try socket(input, action: .gamepad("gamepadA"))
        let b = try socket(input, action: .gamepad("gamepadB"))
        input.process([sample(1, .began, a.center), sample(1, .moved, b.center)])
        XCTAssertEqual(input.gamepad.buttonMask, 32)
        input.process([sample(1, .moved, CGPoint(x: 400, y: 30))])
        XCTAssertEqual(input.gamepad, .neutral)
    }

    func testForzaLayerCancelsReplacedControlAndRestoresBase() throws {
        let input = try engine("forza-layouts")
        let layer = try socket(input, action: .layer("dPadDisplay"))
        let gas = try XCTUnwrap(input.sockets.first { $0.id == "right/inner/0" })
        input.process([sample(1, .began, gas.center), sample(2, .began, layer.center)])
        XCTAssertEqual(input.activeLayers, ["dPadDisplay"])
        XCTAssertEqual(input.gamepad.rightTrigger, 0)
        let pad = try XCTUnwrap(input.sockets.first { $0.id == "right/inner/0" })
        XCTAssertEqual(pad.control.kind, .directionalPad)
        let up = CGPoint(x: pad.center.x, y: pad.center.y - pad.collisionRadius / 2)
        input.process([sample(3, .began, up)])
        XCTAssertEqual(input.gamepad.buttonMask & 256, 256)
        input.process([sample(2, .ended, layer.center)])
        XCTAssertTrue(input.activeLayers.isEmpty)
        XCTAssertEqual(input.gamepad, .neutral)
        XCTAssertEqual(input.sockets.first { $0.id == "right/inner/0" }?.control.kind, .button)
    }

    func testLayerHasMultipleIndependentSources() throws {
        let input = try engine("forza-layouts")
        let button = try socket(input, action: .layer("dPadDisplay"))
        input.process([
            sample(1, .began, button.center), sample(2, .began, button.center),
            sample(1, .ended, button.center),
        ])
        XCTAssertEqual(input.activeLayers, ["dPadDisplay"])
        input.process([sample(2, .ended, button.center)])
        XCTAssertTrue(input.activeLayers.isEmpty)
    }

    func testThrottleStickyAndBrakeRelease() throws {
        let input = try engine("forza-layouts", index: 1)
        let pedal = try XCTUnwrap(input.sockets.first { $0.control.kind == .throttle })
        let up = CGPoint(x: pedal.center.x, y: pedal.center.y - pedal.collisionRadius * 0.8)
        input.process([
            sample(1, .began, pedal.center), sample(1, .moved, up), sample(1, .ended, up),
        ])
        XCTAssertEqual(input.gamepad.rightTrigger, 0.8, accuracy: 0.001)
        XCTAssertEqual(input.gamepad.leftTrigger, 0)
        let down = CGPoint(x: pedal.center.x, y: pedal.center.y + pedal.collisionRadius * 1.6)
        input.process([sample(2, .began, pedal.center), sample(2, .moved, down)])
        XCTAssertEqual(input.gamepad.rightTrigger, 0)
        XCTAssertGreaterThan(input.gamepad.leftTrigger, 0.7)
        input.process([sample(2, .ended, down)])
        XCTAssertEqual(input.gamepad, .neutral)
    }

    func testLayoutViewportAndBackgroundCancelEverything() throws {
        let input = try engine("forza-layouts", index: 1)
        let pedal = try XCTUnwrap(input.sockets.first { $0.control.kind == .throttle })
        let up = CGPoint(x: pedal.center.x, y: pedal.center.y - pedal.collisionRadius)
        for reason in ["background", "pageClosed", "layoutChanged"] {
            input.process([sample(1, .began, pedal.center), sample(1, .moved, up)])
            input.cancelAll(reason: reason)
            XCTAssertEqual(input.gamepad, .neutral)
            XCTAssertTrue(input.runtimes.values.allSatisfy { !$0.isActive })
        }
        input.process([
            sample(1, .began, pedal.center), sample(1, .moved, up), sample(1, .ended, up),
        ])
        input.setViewport(size: CGSize(width: 420, height: 800))
        XCTAssertEqual(input.gamepad, .neutral)
        input.load(try TouchLayoutCatalog.read("generic")[0])
        XCTAssertEqual(input.gamepad, .neutral)
    }

    func testInvalidCoordinatesAreDiscardedAndPressureIsFinite() throws {
        let input = try engine()
        let a = try socket(input, action: .gamepad("gamepadA"))
        var frame: TouchInputFrame?
        input.onInput = { frame = $0 }
        input.process([
            sample(1, .began, CGPoint(x: CGFloat.nan, y: 0)),
            .init(id: 2, phase: .began, position: a.center, pressure: .infinity),
        ])
        XCTAssertEqual(frame?.pointerCount, 1)
        XCTAssertEqual(frame?.events.first?.pressure, 0)
    }

    func testRecorderExportsEdgesAndNeutralReset() throws {
        let recorder = TouchLayoutRecorder()
        recorder.input.setViewport(size: CGSize(width: 800, height: 420))
        let a = try socket(recorder.input, action: .gamepad("gamepadA"))
        recorder.input.process([sample(1, .began, a.center), sample(1, .ended, a.center)])
        let root = try XCTUnwrap(
            JSONSerialization.jsonObject(with: recorder.export()) as? [String: Any])
        XCTAssertEqual(root["version"] as? Int, 2)
        XCTAssertEqual(root["clock"] as? String, "systemUptimeSeconds")
        XCTAssertNotNil(root["sampling"])
        XCTAssertGreaterThan((root["frames"] as? [Any])?.count ?? 0, 0)
        recorder.refreshPresentation()
        XCTAssertTrue(recorder.recentEvents.contains { $0.contains("A ended") })
        recorder.clear()
        XCTAssertEqual(recorder.latest?.gamepad, .neutral)
        XCTAssertEqual(recorder.frames.count, 1)
    }

    func testAbsoluteJoystickAndExclusivePointer() throws {
        let input = try engine("forza-layouts")
        let stick = try XCTUnwrap(input.sockets.first { $0.id == "left/outer/2/1" })
        let right = CGPoint(x: stick.center.x + stick.unitSize * 0.2, y: stick.center.y)
        input.process([sample(1, .began, right), sample(2, .began, stick.center)])
        XCTAssertGreaterThan(input.gamepad.rightThumbX, 0.1)
        XCTAssertEqual(input.runtimes[stick.id]?.pointers.count, 1)
        input.process([sample(2, .ended, stick.center)])
        XCTAssertGreaterThan(input.gamepad.rightThumbX, 0.1)
        input.process([sample(1, .ended, right)])
        XCTAssertEqual(input.gamepad, .neutral)
    }

    func testCapabilityReportIncludesBehaviorsAndUnknownAxisPath() throws {
        let forza = try XCTUnwrap(TouchLayoutCatalog.read("forza-layouts").last)
        XCTAssertTrue(forza.report.behaviors.contains("sticky"))
        XCTAssertTrue(forza.report.behaviors.contains("layer"))
        XCTAssertTrue(forza.report.behaviors.contains("cluster"))
        let data = Data(
            #"{"content":{"left":{"inner":[{"type":"joystick","axis":{"input":"axisXY","output":"leftTrigger","unknownField":1}}]}}}"#
                .utf8)
        let report = try XCTUnwrap(TouchLayoutNormalizer.decode(data).first?.report)
        XCTAssertFalse(report.isSupported)
        XCTAssertTrue(
            report.issues.contains { $0.path == "/content/left/inner/0/axis/unknownField" })
    }

    func testTogglePersistsUntilNextTapAndCancelReleasesIt() throws {
        let data = Data(
            #"{"content":{"left":{"inner":[{"type":"button","action":"gamepadA","toggle":true}]}}}"#
                .utf8)
        let layout = try XCTUnwrap(TouchLayoutNormalizer.decode(data).first)
        let input = try engine()
        input.load(layout)
        let a = try socket(input, action: .gamepad("gamepadA"))
        input.process([sample(1, .began, a.center), sample(1, .ended, a.center)])
        XCTAssertEqual(input.gamepad.buttonMask, 16)
        input.process([sample(2, .began, a.center), sample(2, .ended, a.center)])
        XCTAssertEqual(input.gamepad, .neutral)
        input.process([sample(3, .began, a.center), sample(3, .ended, a.center)])
        input.cancelAll()
        XCTAssertEqual(input.gamepad, .neutral)
    }

    func testSurfaceRendersTransparentLayersAndCancelPublishesNeutral() throws {
        let input = try engine()
        let surface = TouchLayoutSurfaceView(input: input)
        surface.frame = CGRect(x: 0, y: 0, width: 800, height: 420)
        surface.layoutIfNeeded()
        surface.requestRender()
        XCTAssertTrue(surface.isMultipleTouchEnabled)
        XCTAssertFalse(surface.isOpaque)
        XCTAssertEqual(surface.layer.sublayers?.count, input.sockets.count)
        let format = UIGraphicsImageRendererFormat()
        format.opaque = false
        format.scale = 1
        let image = UIGraphicsImageRenderer(size: surface.bounds.size, format: format).image {
            surface.layer.render(in: $0.cgContext)
        }
        let cgImage = try XCTUnwrap(image.cgImage)
        var pixel = [UInt8](repeating: 0, count: 4)
        let context = try XCTUnwrap(
            CGContext(
                data: &pixel, width: 1, height: 1,
                bitsPerComponent: 8, bytesPerRow: 4,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.draw(cgImage, in: CGRect(x: -400, y: -210, width: 800, height: 420))
        XCTAssertEqual(pixel[3], 0, "画面中心保持透明")
        let attachment = XCTAttachment(image: image)
        attachment.name = "generic-transparent-surface"
        attachment.lifetime = .keepAlways
        add(attachment)
        let a = try socket(input, action: .gamepad("gamepadA"))
        input.process([sample(1, .began, a.center)])
        surface.cancel(reason: "background")
        XCTAssertEqual(input.gamepad, .neutral)
        surface.stop()
    }

    func testShortTapFeedbackSurvivesNeutralInputUntilMinimumDuration() throws {
        let input = try engine()
        let a = try socket(input, action: .gamepad("gamepadA"))
        var latest: TouchInputFrame?
        input.onInput = { latest = $0 }
        input.process([sample(1, .began, a.center), sample(1, .ended, a.center)])
        var feedback = TouchPressFeedback()
        let frame = try XCTUnwrap(latest)
        XCTAssertEqual(feedback.update(frame, now: 10), [a.id])
        XCTAssertEqual(input.gamepad, .neutral)
        XCTAssertTrue(feedback.isHighlighted(a.id, now: 10.05))
        XCTAssertTrue(feedback.needsDisplayFrame(now: 10.05))
        XCTAssertFalse(feedback.isHighlighted(a.id, now: 10.15))
        XCTAssertFalse(feedback.needsDisplayFrame(now: 10.15))
    }

    func testHeldFeedbackPersistsAndCancellationClearsIt() throws {
        let input = try engine()
        let a = try socket(input, action: .gamepad("gamepadA"))
        var latest: TouchInputFrame?
        input.onInput = { latest = $0 }
        var feedback = TouchPressFeedback()
        input.process([sample(1, .began, a.center)])
        _ = feedback.update(try XCTUnwrap(latest), now: 10)
        XCTAssertTrue(feedback.isHighlighted(a.id, now: 20))
        XCTAssertFalse(feedback.needsDisplayFrame(now: 20))
        input.process([sample(1, .ended, a.center)])
        _ = feedback.update(try XCTUnwrap(latest), now: 20)
        XCTAssertFalse(feedback.isHighlighted(a.id, now: 20))
        input.process([sample(2, .began, a.center), sample(2, .ended, a.center)])
        _ = feedback.update(try XCTUnwrap(latest), now: 30)
        input.cancelAll(reason: "background")
        _ = feedback.update(try XCTUnwrap(latest), now: 30.01)
        XCTAssertFalse(feedback.isHighlighted(a.id, now: 30.01))
        XCTAssertFalse(feedback.needsDisplayFrame(now: 30.01))
    }

    func testShortTapAndHeldPressDrawVisibleFeedbackAndKeepInputReleaseImmediate() throws {
        let input = try engine()
        let surface = TouchLayoutSurfaceView(input: input)
        surface.frame = CGRect(x: 0, y: 0, width: 800, height: 420)
        surface.layoutIfNeeded()
        surface.controlOpacity = 0.2
        surface.requestRender()
        let a = try socket(input, action: .gamepad("gamepadA"))
        let probe = CGPoint(x: a.center.x - a.diameter * 0.2, y: a.center.y - a.diameter * 0.2)
        let idle = renderSurface(surface)
        var presses = 0
        surface.onPress = { presses += 1 }
        input.process([sample(1, .began, a.center)])
        let held = renderSurface(surface)
        XCTAssertGreaterThan(try alpha(held, at: probe), try alpha(idle, at: probe) + 25)
        XCTAssertEqual(input.gamepad.buttonMask, 16)
        surface.cancel(reason: "reset")
        input.process([sample(2, .began, a.center), sample(2, .ended, a.center)])
        let tapped = renderSurface(surface)
        XCTAssertEqual(input.gamepad, .neutral)
        XCTAssertGreaterThan(try alpha(tapped, at: probe), try alpha(idle, at: probe) + 25)
        XCTAssertEqual(presses, 2)
        surface.cancel(reason: "background")
        XCTAssertEqual(try alpha(renderSurface(surface), at: probe), try alpha(idle, at: probe))
        for (name, image) in [
            ("feedback-idle", idle), ("feedback-held", held), ("feedback-short-tap", tapped),
        ] {
            let attachment = XCTAttachment(image: image)
            attachment.name = name
            attachment.lifetime = .keepAlways
            add(attachment)
        }
        surface.stop()
    }

    func testCoalescedHistoryUsesOriginalIdentityAndKeepsTerminalOnce() throws {
        let input = try engine()
        let a = try socket(input, action: .gamepad("gamepadA"))
        let base = ProcessInfo.processInfo.systemUptime - 1
        let current = TouchPointerSample(
            id: 7, phase: .ended, position: a.center,
            sourceTimestamp: base + 0.03)
        let history = [0.02, 0.01, 0.03].map {
            TouchPointerSample(
                id: 999, phase: .ended, position: a.center,
                sourceTimestamp: base + $0)
        }
        let batch = TouchSampleBatch.coalescing(current: current, history: history)
        XCTAssertEqual(batch.map(\.id), [7, 7, 7])
        XCTAssertEqual(batch.map(\.phase), [.moved, .moved, .ended])
        XCTAssertEqual(batch.map(\.isCoalesced), [true, true, false])
        XCTAssertEqual(
            batch.compactMap(\.sourceTimestamp), [base + 0.01, base + 0.02, base + 0.03])
        input.process([.init(id: 7, phase: .began, position: a.center, sourceTimestamp: base)])
        var frame: TouchInputFrame?
        input.onInput = { frame = $0 }
        input.process(batch)
        XCTAssertEqual(frame?.events.map(\.phase), [.moved, .moved, .ended])
        XCTAssertEqual(input.gamepad, .neutral)
        XCTAssertEqual(input.diagnostics.summary.coalescedSamples, 2)
        let began = TouchPointerSample(
            id: 8, phase: .began, position: a.center, sourceTimestamp: base)
        XCTAssertEqual(TouchSampleBatch.coalescing(current: began, history: history).count, 1)
    }

    func testDuplicateAndStaleMovesAreCountedAndAbnormalTerminalReleases() throws {
        let input = try engine()
        let a = try socket(input, action: .gamepad("gamepadA"))
        let base = ProcessInfo.processInfo.systemUptime - 1
        var frame: TouchInputFrame?
        input.onInput = { frame = $0 }
        input.process([
            .init(id: 1, phase: .began, position: a.center, sourceTimestamp: base),
            .init(id: 1, phase: .moved, position: a.center, sourceTimestamp: base + 0.02),
            .init(id: 1, phase: .moved, position: .zero, sourceTimestamp: base + 0.02),
            .init(id: 1, phase: .moved, position: .zero, sourceTimestamp: base + 0.01),
            .init(
                id: 1, phase: .ended, position: CGPoint(x: CGFloat.nan, y: 0),
                sourceTimestamp: .nan),
        ])
        XCTAssertEqual(frame?.events.map(\.phase), [.began, .moved, .ended])
        XCTAssertEqual(frame?.transitions.map(\.buttonMask), [16, 0])
        XCTAssertEqual(input.gamepad, .neutral)
        let stats = input.diagnostics.summary
        XCTAssertEqual(stats.duplicateSamples, 1)
        XCTAssertEqual(stats.outOfOrderSamples, 1)
        XCTAssertEqual(stats.invalidSamples, 2)
        XCTAssertEqual(stats.beganCount, 1)
        XCTAssertEqual(stats.endedCount, 1)
        input.process([
            .init(id: 2, phase: .began, position: a.center, sourceTimestamp: base + 0.02),
            .init(id: 2, phase: .cancelled, position: a.center, sourceTimestamp: base + 0.01),
        ])
        XCTAssertEqual(input.gamepad, .neutral)
        XCTAssertEqual(input.diagnostics.summary.cancelledCount, 1)
    }

    func testSourceTimingAndCallbackTimingRemainIndependentForMultiplePointers() throws {
        for hz in [60, 120, 240] {
            var now: TimeInterval = 100
            let input = TouchInputAccumulator(clock: { now + 0.0002 })
            input.setViewport(size: CGSize(width: 800, height: 420))
            input.load(try TouchLayoutCatalog.read("generic")[0])
            input.resetDiagnostics()
            let stick = try XCTUnwrap(input.sockets.first { $0.id == "left/inner/0" })
            let a = try socket(input, action: .gamepad("gamepadA"))
            input.process(
                [
                    .init(id: 1, phase: .began, position: stick.center, sourceTimestamp: 100),
                    .init(id: 2, phase: .began, position: a.center, sourceTimestamp: 100),
                ], timestamp: now)
            let perCallback = hz / 60
            for callback in 1...60 {
                now = 100 + Double(callback) / 60 + 0.001
                var batch: [TouchPointerSample] = []
                for offset in 1...perCallback {
                    let index = (callback - 1) * perCallback + offset
                    let source = 100 + Double(index) / Double(hz)
                    for (id, position) in [(1, stick.center), (2, a.center)] {
                        batch.append(
                            .init(
                                id: id, phase: .moved, position: position,
                                sourceTimestamp: source, isCoalesced: offset < perCallback))
                    }
                }
                input.process(batch, timestamp: now)
            }
            let stats = input.diagnostics.summary
            XCTAssertEqual(stats.movementIntervals.frequencyHz, Double(hz), accuracy: 0.00001)
            XCTAssertEqual(stats.pointers.count, 2)
            for pointer in stats.pointers {
                XCTAssertEqual(pointer.movementIntervals.frequencyHz, Double(hz), accuracy: 0.00001)
            }
            XCTAssertEqual(stats.callbackIntervals.frequencyHz, 60, accuracy: 0.1)
            XCTAssertEqual(stats.acceptedSamples, hz * 2 + 2)
            XCTAssertEqual(stats.coalescedSamples, (hz - 60) * 2)
            XCTAssertEqual(
                stats.movementIntervals.p99Milliseconds, 1000 / Double(hz), accuracy: 0.001)
            XCTAssertGreaterThan(
                stats.sourceToState.meanMilliseconds, stats.sourceToReceipt.meanMilliseconds)
            input.cancelAll(reason: "background")
            XCTAssertEqual(input.gamepad, .neutral)
            XCTAssertEqual(input.diagnostics.summary.cancelledCount, 2)
            XCTAssertEqual(input.diagnostics.summary.synthesizedCancelCount, 2)
            XCTAssertEqual(input.diagnostics.summary.acceptedSamples, hz * 2 + 2)
        }
    }

    func testRecorderRetainsOrderedEdgesAtCapacityAndDefersPresentation() throws {
        let recorder = TouchLayoutRecorder()
        recorder.input.setViewport(size: CGSize(width: 800, height: 420))
        recorder.clear()
        let a = try socket(recorder.input, action: .gamepad("gamepadA"))
        let previousPresentations = recorder.presentationCount
        for id in 1...2300 {
            recorder.input.process([sample(id, .began, a.center), sample(id, .ended, a.center)])
        }
        XCTAssertEqual(recorder.presentationCount, previousPresentations)
        XCTAssertEqual(recorder.frames.count, TouchLayoutRecorder.maximumFrames)
        XCTAssertEqual(recorder.frames.first?.events.first?.pointerID, 253)
        XCTAssertEqual(recorder.frames.last?.events.map(\.phase), [.began, .ended])
        XCTAssertEqual(recorder.frames.last?.transitions.map(\.buttonMask), [16, 0])
        recorder.refreshPresentation()
        XCTAssertEqual(recorder.evictedFrameCount, 253)
        XCTAssertEqual(recorder.retainedFrameCount, 2048)
        XCTAssertEqual(recorder.sampling?.beganCount, 2300)
        XCTAssertEqual(recorder.sampling?.endedCount, 2300)
        XCTAssertEqual(recorder.sampling?.pointers.count, 12)
        let root = try XCTUnwrap(
            JSONSerialization.jsonObject(with: recorder.export()) as? [String: Any])
        XCTAssertEqual(root["evictedFrames"] as? Int, 253)
        XCTAssertEqual(root["metricWindowCapacity"] as? Int, 4096)
        let last = try XCTUnwrap((root["frames"] as? [[String: Any]])?.last)
        XCTAssertEqual((last["transitionTimestamps"] as? [Double])?.count, 2)
        recorder.clear()
        XCTAssertEqual(recorder.evictedFrameCount, 0)
        XCTAssertEqual(recorder.input.gamepad, .neutral)
    }

    func testTimingWindowBoundsAndNearestRankPercentiles() {
        var window = TouchTimingWindow()
        for index in 1...5000 { window.record(Double(index) / 1000) }
        let summary = window.summary
        XCTAssertEqual(summary.totalCount, 5000)
        XCTAssertEqual(summary.windowCount, 4096)
        XCTAssertEqual(summary.maximumMilliseconds, 5000)
        XCTAssertEqual(summary.p95Milliseconds, 4796)
        XCTAssertEqual(summary.p99Milliseconds, 4960)
        var ring = TouchRingBuffer<Int>(capacity: 3)
        for index in 1...5 { ring.append(index) }
        XCTAssertEqual(ring.elements, [3, 4, 5])
        XCTAssertEqual(ring.evictedCount, 2)
    }

    func testHighRateStickThrottleAndShortTapReplayPreservesEdges() throws {
        struct Result: Encodable {
            let sourceHz: Int
            let callbacks: Int
            let offeredSamples: Int
            let beganEdges: Int
            let terminalEdges: Int
            let elapsedMilliseconds: Double
            let processing: TouchTimingSummary
        }
        var results: [Result] = []
        for hz in [60, 120, 240] {
            let input = try engine("forza-layouts", index: 1)
            let stick = try XCTUnwrap(input.sockets.first { $0.control.kind == .joystick })
            let pedal = try XCTUnwrap(input.sockets.first { $0.control.kind == .throttle })
            let a = try socket(input, action: .gamepad("gamepadA"))
            var begins = 0
            var terminals = 0
            var samples = 0
            input.onInput = { frame in
                begins += frame.events.filter { $0.phase == .began }.count
                terminals +=
                    frame.events.filter { $0.phase == .ended || $0.phase == .cancelled }.count
                samples += frame.events.count
                XCTAssertEqual(frame.transitions.count, frame.transitionTimestamps.count)
            }
            input.resetDiagnostics()
            let start = ProcessInfo.processInfo.systemUptime
            input.process([sample(1, .began, stick.center), sample(2, .began, pedal.center)])
            let perCallback = hz / 60
            for callback in 1...600 {
                var batch: [TouchPointerSample] = []
                for offset in 1...perCallback {
                    let index = (callback - 1) * perCallback + offset
                    let fraction = Double(index % 60) / 60
                    batch.append(
                        sample(
                            1, .moved,
                            CGPoint(
                                x: stick.center.x + cos(fraction * .pi * 2) * 30,
                                y: stick.center.y + sin(fraction * .pi * 2) * 30)))
                    batch.append(
                        sample(
                            2, .moved,
                            CGPoint(
                                x: pedal.center.x,
                                y: pedal.center.y - pedal.collisionRadius * fraction)))
                }
                batch.append(sample(callback + 2, .began, a.center))
                batch.append(sample(callback + 2, .ended, a.center))
                input.process(batch)
                XCTAssertGreaterThanOrEqual(input.gamepad.rightTrigger, 0)
                XCTAssertLessThanOrEqual(input.gamepad.rightTrigger, 1)
            }
            input.cancelAll(reason: "background")
            let elapsed = (ProcessInfo.processInfo.systemUptime - start) * 1000
            let stats = input.diagnostics.summary
            XCTAssertEqual(begins, 602)
            XCTAssertEqual(terminals, begins)
            XCTAssertEqual(samples, hz * 10 * 2 + 1204)
            XCTAssertEqual(input.gamepad, .neutral)
            XCTAssertEqual(
                stats.invalidSamples + stats.duplicateSamples + stats.outOfOrderSamples, 0)
            XCTAssertLessThan(
                stats.processing.p99Milliseconds, 8,
                "处理 P99 应低于现有 8ms 输入轮询预算")
            results.append(
                .init(
                    sourceHz: hz, callbacks: stats.callbackCount,
                    offeredSamples: stats.offeredSamples, beganEdges: begins,
                    terminalEdges: terminals,
                    elapsedMilliseconds: elapsed, processing: stats.processing))
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        let attachment = XCTAttachment(
            data: try encoder.encode(results), uniformTypeIdentifier: "public.json")
        attachment.name = "sampling-pressure-replay"
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func renderSurface(_ surface: UIView) -> UIImage {
        let format = UIGraphicsImageRendererFormat()
        format.opaque = false
        format.scale = 1
        return UIGraphicsImageRenderer(size: surface.bounds.size, format: format).image {
            surface.layer.render(in: $0.cgContext)
        }
    }

    func testForzaCameraHasPressFeedbackWithTransparentDefaultOutline() throws {
        let input = try engine("forza-layouts")
        let surface = TouchLayoutSurfaceView(input: input)
        surface.frame = CGRect(x: 0, y: 0, width: 800, height: 420)
        surface.layoutIfNeeded()
        let camera = try XCTUnwrap(input.sockets.first { $0.id == "left/outer/2/1" })
        let probe = CGPoint(
            x: camera.center.x - camera.diameter * 0.2,
            y: camera.center.y - camera.diameter * 0.2)
        let idle = renderSurface(surface)
        input.process([sample(1, .began, camera.center)])
        let pressed = renderSurface(surface)
        XCTAssertGreaterThan(try alpha(pressed, at: probe), try alpha(idle, at: probe) + 25)
        surface.cancel(reason: "background")
        XCTAssertEqual(input.gamepad, .neutral)
        surface.stop()
    }

    private func alpha(_ image: UIImage, at point: CGPoint) throws -> Int {
        let pixelImage = try XCTUnwrap(
            image.cgImage?.cropping(to: CGRect(x: point.x, y: point.y, width: 1, height: 1)))
        var pixel = [UInt8](repeating: 0, count: 4)
        let context = try XCTUnwrap(
            CGContext(
                data: &pixel, width: 1, height: 1, bitsPerComponent: 8,
                bytesPerRow: 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.draw(pixelImage, in: CGRect(x: 0, y: 0, width: 1, height: 1))
        return Int(pixel[3])
    }
}
