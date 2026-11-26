import UIKit
import XCTest

@testable import XBXRC

@MainActor
final class TouchLayoutCompletenessTests: XCTestCase {
  private let dynamicDocument =
    ##"{"_xbxrcRuntime":{"layout":{"content":{"right":{"inner":[{"type":"button","action":"gamepadA","visible":{"$ref":"runtime-context.json#/state/fireVisible"}},{"type":"button","action":"gamepadB"}]}}},"context":{"state":{"fireVisible":true}},"assets":{}}}"##

  private func decode(_ source: String, patch: String? = nil) throws -> TouchLayout {
    try XCTUnwrap(TouchLayoutNormalizer.decode(Data(source.utf8), patchJSON: patch).first)
  }

  func testDynamicPatchReleasesHiddenControlAndPreservesOtherPointer() throws {
    let input = TouchInputAccumulator()
    input.setViewport(size: CGSize(width: 800, height: 420))
    let layout = try decode(dynamicDocument)
    XCTAssertTrue(layout.report.isSupported)
    input.load(layout)
    let a = try XCTUnwrap(input.sockets.first { $0.control.action == .gamepad("gamepadA") })
    let b = try XCTUnwrap(input.sockets.first { $0.control.action == .gamepad("gamepadB") })
    input.process([
      .init(id: 1, phase: .began, position: a.center),
      .init(id: 2, phase: .began, position: b.center),
    ])
    XCTAssertEqual(input.gamepad.buttonMask, 48)
    let updated = try decode(
      try XCTUnwrap(layout.runtimeDocument),
      patch: ##"[{"op":"replace","path":"/fireVisible","value":false}]"##)
    input.updateLayout(updated)
    XCTAssertEqual(input.gamepad.buttonMask, 32)
    XCTAssertNil(input.runtimes[a.id])
    XCTAssertEqual(input.runtimes[b.id]?.pointers.count, 1)
    input.process([.init(id: 2, phase: .ended, position: b.center)])
    XCTAssertEqual(input.gamepad, .neutral)
    let invalid = try decode(
      try XCTUnwrap(updated.runtimeDocument),
      patch: ##"[{"op":"replace","path":"/fireVisible","value":"yes"}]"##)
    XCTAssertFalse(invalid.report.isSupported)
  }

  func testTurboUsesMillisecondsAndSharedActionsReleaseIndependently() throws {
    let input = TouchInputAccumulator(clock: { 10 })
    input.setViewport(size: CGSize(width: 800, height: 420))
    let layout = try decode(
      ##"{"content":{"right":{"inner":[{"type":"button","action":{"type":"turbo","action":"gamepadA","interval":100}},{"type":"button","action":"gamepadA"}]}}}"##
    )
    XCTAssertTrue(layout.report.isSupported)
    input.load(layout)
    let turbo = try XCTUnwrap(input.sockets.first { $0.control.action?.containsTurbo == true })
    let steady = try XCTUnwrap(input.sockets.first { $0.control.action == .gamepad("gamepadA") })
    input.process(
      [.init(id: 1, phase: .began, position: turbo.center, sourceTimestamp: 10)], timestamp: 10)
    XCTAssertEqual(input.gamepad.buttonMask, 16)
    XCTAssertTrue(input.needsTimeUpdate)
    input.advance(at: 10.101)
    XCTAssertEqual(input.gamepad, .neutral)
    input.advance(at: 10.201)
    XCTAssertEqual(input.gamepad.buttonMask, 16)
    input.process([.init(id: 2, phase: .began, position: steady.center)], timestamp: 10.201)
    input.advance(at: 10.301)
    XCTAssertEqual(input.gamepad.buttonMask, 16)
    input.process([.init(id: 2, phase: .ended, position: steady.center)], timestamp: 10.301)
    XCTAssertEqual(input.gamepad, .neutral)
    input.cancelAll(reason: "inputSuppressed")
    XCTAssertFalse(input.needsTimeUpdate)
    input.advance(at: 10.401)
    XCTAssertEqual(input.gamepad, .neutral)
  }

  func testNamedColorsHighContrastAndDirectionalGradientRender() throws {
    let layout = try decode(
      ##"{"styles":{"colors":{"default":{"accent":"#0f0","system_contentPrimary":"#f00"},"highContrast":{"accent":"#ff0"}}},"content":{"left":{"inner":[{"type":"directionalPad","styles":{"default":{"fill":"colors/accent","gradient":{"color":"colors/accent"},"stroke":{"type":"solid","color":"colors/system_contentPrimary","opacity":0.6}}}}]}}}"##
    )
    XCTAssertTrue(layout.report.isSupported, "\(layout.report.issues)")
    XCTAssertEqual(TouchStylePalette.hex("colors/accent", palette: layout.colorPalette), "#0f0")
    XCTAssertEqual(
      TouchStylePalette.hex("colors/accent", palette: layout.colorPalette, highContrast: true),
      "#ff0")
    XCTAssertEqual(
      TouchStylePalette.hex("colors/system_contentPrimary", palette: layout.colorPalette), "#f00")
    let input = TouchInputAccumulator()
    input.load(layout)
    let view = TouchLayoutSurfaceView(input: input)
    view.frame = CGRect(x: 0, y: 0, width: 800, height: 420)
    view.updateViewport()
    let socket = try XCTUnwrap(input.sockets.first)
    input.process([
      .init(
        id: 1, phase: .began,
        position: CGPoint(x: socket.center.x + socket.diameter * 0.3, y: socket.center.y))
    ])
    view.requestRender()
    let gradient = try XCTUnwrap(
      view.layer.sublayers?.flatMap { $0.sublayers ?? [] }.first { $0.name == "direction-gradient" }
        as? CAGradientLayer)
    XCTAssertFalse(gradient.isHidden)
    XCTAssertEqual(gradient.colors?.count, 2)
    XCTAssertGreaterThan(gradient.endPoint.x, gradient.startPoint.x)
    let attachment = XCTAttachment(
      image: UIGraphicsImageRenderer(bounds: view.bounds).image { context in
        view.layer.render(in: context.cgContext)
      })
    attachment.name = "tak-directional-gradient"
    attachment.lifetime = .keepAlways
    add(attachment)
    input.cancelAll()
    view.requestRender()
    XCTAssertTrue(gradient.isHidden)
    view.stop()
  }

  func testDeferredInputsMissingColorsAndUnknownStylesKeepCapabilityGate() throws {
    for source in [
      ##"{"content":{"right":{"inner":[{"type":"button","action":"mouseLeft"}]}}}"##,
      ##"{"content":{"right":{"inner":[{"type":"button","action":{"type":"keyboard","keyCode":65}}]}}}"##,
      ##"{"content":{"sensors":[{"type":"gyroscope","axis":{"input":"axisXY","output":"rightJoystick"}}]}}"##,
      ##"{"content":{"right":{"inner":[{"type":"button","action":"gamepadA","styles":{"default":{"background":{"type":"color","value":"colors/missing"}}}}]}}}"##,
      ##"{"content":{"right":{"inner":[{"type":"button","action":"gamepadA","styles":{"future":{"background":{"type":"color","value":"#fff"}}}}]}}}"##,
    ] {
      XCTAssertTrue(try decode(source).report.isRunnable, source)
      XCTAssertFalse(try decode(source).report.isFullyCompatible, source)
    }
  }

  func testEveryOfficialIconHasNativeArtworkAndCommunityReferencesResolve() throws {
    XCTAssertEqual(TouchIconAtlas.names.count, 234)
    for name in TouchProtocolCatalog.information.iconNames {
      XCTAssertNotNil(TouchIconAtlas.artwork(name)?.cgImage, name)
    }
    let source =
      ##"{"schema_version":1,"layouts":{"community":{"definitions":{"common":{"type":"button","action":"gamepadA","styles":{"default":{"faceImage":{"type":"icon","value":"inventory","label":{"type":"action"}}}}}},"content":{"right":{"inner":[{"$ref":"#/definitions/common"}]}}}}}"##
    let layout = try decode(source)
    XCTAssertEqual(layout.report.format, "community")
    XCTAssertTrue(layout.report.isSupported, "\(layout.report.issues)")
    XCTAssertEqual(layout.content["right"]?["inner"]?.array?.first?["action"]?.string, "gamepadA")
  }

  func testRealCommunityProtocolUsesCompiledInputsAndReleasesAllContributions() throws {
    let layout = try XCTUnwrap(TouchLayoutCatalog.read("gta-vi-layouts").first)
    XCTAssertTrue(layout.report.isRunnable)
    XCTAssertEqual(layout.compiledControls.count, 15)
    XCTAssertEqual(layout.protocolRevision, TouchProtocolCatalog.information.revision)
    let input = TouchInputAccumulator(clock: { 10 })
    input.setViewport(size: CGSize(width: 800, height: 420))
    input.load(layout)
    let trigger = try XCTUnwrap(
      input.sockets.first { $0.control.action == .gamepad("leftTrigger") })
    let pad = try XCTUnwrap(input.sockets.first { $0.control.kind == .touchpad })
    XCTAssertNotNil(trigger.control.compiled)
    XCTAssertNotNil(pad.control.compiled)
    input.process(
      [
        .init(id: 1, phase: .began, position: trigger.center, sourceTimestamp: 10),
        .init(id: 2, phase: .began, position: pad.center, sourceTimestamp: 10),
      ], timestamp: 10)
    input.process(
      [
        .init(
          id: 1, phase: .moved,
          position: CGPoint(x: trigger.center.x + trigger.unitSize, y: trigger.center.y),
          sourceTimestamp: 10.01),
        .init(
          id: 2, phase: .moved, position: CGPoint(x: pad.center.x + 6, y: pad.center.y),
          sourceTimestamp: 10.01),
      ], timestamp: 10.01)
    XCTAssertEqual(input.gamepad.leftTrigger, 1)
    XCTAssertEqual(input.gamepad.rightTrigger, 1)
    XCTAssertGreaterThan(input.gamepad.rightThumbX, 0)
    input.cancelAll()
    XCTAssertEqual(input.gamepad, .neutral)
    for size in [CGSize(width: 393, height: 500), CGSize(width: 800, height: 420)] {
      input.setViewport(size: size)
      for socket in input.sockets {
        XCTAssertEqual(
          TouchLayoutGeometry.hitTest(socket.center, sockets: input.sockets)?.id, socket.id)
      }
    }
  }

  func testUnknownVisualAndInputExtensionsKeepKnownButtonRunnable() throws {
    let layout = try decode(
      ##"{"content":{"right":{"inner":[{"type":"button","action":["gamepadA","mouseLeft"],"styles":{"default":{"faceImage":{"type":"icon","value":"futureIcon"},"futureEffect":true}}}]}}}"##
    )
    XCTAssertTrue(layout.report.isRunnable)
    XCTAssertFalse(layout.report.isFullyCompatible)
    XCTAssertTrue(layout.report.issues.contains { $0.capability == "render.icon" })
    XCTAssertTrue(
      layout.report.issues.contains {
        $0.capability == "input.action" && $0.path == "/content/right/inner/0/action/1"
      })
    let input = TouchInputAccumulator()
    input.setViewport(size: CGSize(width: 800, height: 420))
    input.load(layout)
    let socket = try XCTUnwrap(input.sockets.first)
    XCTAssertEqual(socket.control.action, .group([.gamepad("gamepadA")]))
    input.process([.init(id: 1, phase: .began, position: socket.center)])
    XCTAssertEqual(input.gamepad.buttonMask, 16)
    input.process([.init(id: 1, phase: .ended, position: socket.center)])
    XCTAssertEqual(input.gamepad, .neutral)
  }

  func testProtocolNumericFieldsPreserveRuntimeBehaviorAndFiniteOutputs() throws {
    let source =
      ##"{"content":{"left":{"inner":[{"type":"joystick","relative":false,"expand":false,"action":"gamepadA","actionThreshold":2.5,"axis":{"input":"axisXY","output":"leftJoystick","deadzone":{"threshold":-0.3,"radial":false}}}]}}}"##
    let layout = try decode(source)
    XCTAssertTrue(layout.report.isFullyCompatible)
    let input = TouchInputAccumulator()
    input.setViewport(size: CGSize(width: 800, height: 420))
    input.load(layout)
    let stick = try XCTUnwrap(input.sockets.first)
    XCTAssertEqual(stick.control.actionThreshold, 2.5)
    XCTAssertEqual(stick.control.axis.first?.deadzone, -0.3)
    let point = CGPoint(x: stick.center.x + stick.unitSize * 0.25, y: stick.center.y)
    input.process([.init(id: 1, phase: .began, position: point)])
    XCTAssertEqual(input.gamepad.leftThumbX, 0.35, accuracy: 0.0001)
    XCTAssertEqual(input.gamepad.buttonMask, 0)
    input.process([.init(id: 1, phase: .ended, position: point)])
    XCTAssertEqual(input.gamepad, .neutral)
    input.load(try decode(source.replacingOccurrences(of: #""actionThreshold":2.5,"#, with: "")))
    let defaultStick = try XCTUnwrap(input.sockets.first)
    XCTAssertEqual(defaultStick.control.actionThreshold, 0)
    input.process([.init(id: 2, phase: .began, position: defaultStick.center)])
    XCTAssertEqual(input.gamepad.buttonMask, 16)
    input.cancelAll()
    XCTAssertEqual(input.gamepad, .neutral)
    let huge = TouchInputContribution(axes: ["leftJoystickX": Double.greatestFiniteMagnitude])
    XCTAssertEqual(TouchGamepadMapping.state(contributions: [huge, huge]), .neutral)
  }
}
