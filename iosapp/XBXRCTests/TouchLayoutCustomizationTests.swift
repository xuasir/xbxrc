import UIKit
import XCTest

@testable import XBXRC

@MainActor
final class TouchLayoutCustomizationTests: XCTestCase {
    func testTemplatesPersistBothOrientationsAndIndependentAppearance() throws {
        let suite = "XBXRC.TouchTemplates.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = AppSettingsStore(defaults: defaults)
        var draft = TouchLayoutTemplate(name: "我的布局", appearance: .init(opacity: 0.4, scale: 1.3))
        draft.setPosition(CGPoint(x: 0.4, y: 0.3), for: "right/inner/1", orientation: .landscape)
        draft.setPosition(CGPoint(x: 0.6, y: 0.7), for: "right/inner/1", orientation: .portrait)
        XCTAssertTrue(store.mobile.saveTouchTemplate(draft))
        let restored = AppSettingsStore(defaults: defaults)
        XCTAssertEqual(restored.mobile.selectedTouchTemplate, draft)
        XCTAssertEqual(
            restored.mobile.touchConfiguration(for: .landscape).positions["right/inner/1"]?.x, 0.4)
        XCTAssertEqual(
            restored.mobile.touchConfiguration(for: .portrait).positions["right/inner/1"]?.x, 0.6)
        restored.mobile.touchLayoutID = "classic"
        restored.mobile.saveTouchAppearance(.init(opacity: 0.8, scale: 0.9), for: "classic")
        restored.mobile.touchLayoutID = draft.id
        XCTAssertEqual(restored.mobile.selectedTouchAppearance, draft.appearance)
        restored.mobile.saveTouchAppearance(.init(opacity: 0.3, scale: 1.1), for: draft.id)
        let final = AppSettingsStore(defaults: defaults)
        XCTAssertEqual(final.mobile.selectedTouchAppearance, .init(opacity: 0.3, scale: 1.1))
        XCTAssertEqual(final.mobile.touchOpacity, 0.8)
        XCTAssertEqual(final.mobile.touchScale, 0.9)
    }

    func testUpdateCloneDeleteAndLimitPreserveSavedTemplate() {
        var preferences = MobilePreferences()
        var draft = TouchLayoutTemplate()
        XCTAssertTrue(preferences.saveTouchTemplate(draft))
        XCTAssertFalse(preferences.selectedTouchLayoutName.isEmpty)
        draft.name = "修改后的布局"
        draft.setPosition(CGPoint(x: 0.5, y: 0.4), for: "right/inner/1", orientation: .landscape)
        XCTAssertTrue(preferences.saveTouchTemplate(draft))
        XCTAssertEqual(preferences.touchLayoutTemplates.count, 1)
        XCTAssertTrue(preferences.saveTouchTemplate(draft, asNew: true))
        let cloneID = preferences.touchLayoutID
        XCTAssertNotEqual(cloneID, draft.id)
        XCTAssertEqual(preferences.selectedTouchTemplate?.landscape, draft.landscape)
        preferences.deleteTouchTemplate(cloneID)
        XCTAssertEqual(preferences.touchLayoutID, "classic")
        for _ in 1..<TouchLayoutTemplate.maximumCount {
            XCTAssertTrue(preferences.saveTouchTemplate(TouchLayoutTemplate()))
        }
        let full = preferences
        XCTAssertFalse(preferences.saveTouchTemplate(draft, asNew: true))
        XCTAssertEqual(preferences, full)
        XCTAssertTrue(preferences.saveTouchTemplate(draft))
        XCTAssertEqual(preferences.touchLayoutTemplates.count, 20)
    }

    func testCorruptTemplateDoesNotDiscardOtherTemplatesOrOldSettings() throws {
        let template = TouchLayoutTemplate(name: "有效模板")
        let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(template))
        let data = try JSONSerialization.data(withJSONObject: [
            "touchOpacity": 0.4, "invertLeftY": true, "touchLayoutID": template.id,
            "touchLayoutTemplates": [object, ["id": "invalid"], NSNull()],
        ])
        let preferences = try JSONDecoder().decode(MobilePreferences.self, from: data).normalized()
        XCTAssertEqual(preferences.touchLayoutTemplates, [template])
        XCTAssertEqual(preferences.touchLayoutID, template.id)
        XCTAssertEqual(preferences.touchOpacity, 0.4)
        XCTAssertTrue(preferences.invertLeftY)
        let old = try JSONDecoder().decode(
            MobilePreferences.self,
            from: Data(#"{"touchLayoutID":"forza:layout-1","touchOpacity":0.35}"#.utf8)
        ).normalized()
        XCTAssertEqual(old.touchLayoutID, "classic")
        XCTAssertEqual(old.selectedTouchAppearance, .init(opacity: 0.35, scale: 1))
    }

    func testNormalizationFiltersUnknownControlsAndBoundsValues() throws {
        var template = TouchLayoutTemplate(name: "  " + String(repeating: "名", count: 45) + "  ")
        template.landscape = [
            "right/inner/1": TouchPoint(CGPoint(x: -1, y: 3)),
            "right/inner/2": TouchPoint(CGPoint(x: Double.nan, y: 0.3)),
            "unknown": TouchPoint(CGPoint(x: 0.5, y: 0.5)),
        ]
        template.appearance = .init(opacity: .nan, scale: 8)
        var preferences = MobilePreferences()
        preferences.touchLayoutID = template.id
        var unsupported = template
        unsupported.version = 99
        preferences.touchLayoutTemplates = [unsupported, template, template]
        let normalized = preferences.normalized()
        let restored = try XCTUnwrap(normalized.selectedTouchTemplate)
        XCTAssertEqual(normalized.touchLayoutTemplates.count, 1)
        XCTAssertEqual(restored.name.count, 40)
        XCTAssertEqual(restored.landscape, ["right/inner/1": TouchPoint(CGPoint(x: 0, y: 1))])
        XCTAssertEqual(restored.appearance, .init(opacity: 0.65, scale: 1.5))
        preferences.touchLayoutID = UUID().uuidString
        XCTAssertEqual(preferences.normalized().touchLayoutID, "classic")
    }

    func testConfiguredSocketsStayInViewportAndShareHitGeometryAcrossSizes() throws {
        let layout = try TouchLayoutCatalog.standard()
        for size in [
            CGSize(width: 390, height: 844), CGSize(width: 844, height: 390),
            CGSize(width: 834, height: 1194), CGSize(width: 1194, height: 834),
        ] {
            let viewport = StreamingTouchViewportPolicy.frame(in: size)
            for scale in [0.75, 1.0, 1.5] {
                let input = TouchInputAccumulator()
                input.load(layout)
                input.setConfiguration(
                    .init(
                        positions: ["right/inner/1": TouchPoint(CGPoint(x: 0.5, y: 0.2))],
                        scale: scale))
                input.setViewport(size: viewport.size)
                for socket in input.sockets {
                    XCTAssertTrue(
                        CGRect(origin: .zero, size: viewport.size).contains(socket.frame),
                        "\(size) \(socket.id)")
                    XCTAssertGreaterThanOrEqual(socket.collisionRadius, 22)
                    if size.height > size.width {
                        XCTAssertGreaterThanOrEqual(
                            socket.frame.minY + viewport.minY, size.height / 2)
                    }
                }
                let a = try XCTUnwrap(input.sockets.first { $0.id == "right/inner/1" })
                XCTAssertEqual(
                    TouchLayoutGeometry.hitTest(a.center, sockets: input.sockets)?.id, a.id)
                input.process([.init(id: 1, phase: .began, position: a.center)])
                XCTAssertEqual(input.gamepad.buttonMask, 16)
                input.process([.init(id: 1, phase: .ended, position: a.center)])
                XCTAssertEqual(input.gamepad, .neutral)
            }
        }
    }

    func testScalingChangesDrawingAndJoystickUnitAndConfigurationReleasesHeldInput() throws {
        let input = TouchInputAccumulator()
        input.load(try TouchLayoutCatalog.standard())
        input.setViewport(size: CGSize(width: 844, height: 390))
        input.setConfiguration(.init())
        let stick = try XCTUnwrap(input.sockets.first { $0.id == "left/inner/0" })
        let a = try XCTUnwrap(input.sockets.first { $0.id == "right/inner/1" })
        input.process([.init(id: 1, phase: .began, position: a.center)])
        XCTAssertEqual(input.gamepad.buttonMask, 16)
        var last: TouchInputFrame?
        input.onInput = { last = $0 }
        input.setConfiguration(.init(identity: UUID().uuidString, scale: 1.5))
        XCTAssertEqual(input.gamepad, .neutral)
        XCTAssertEqual(last?.pointerCount, 0)
        XCTAssertEqual(last?.reason, "configurationChanged")
        let scaled = try XCTUnwrap(input.sockets.first { $0.id == stick.id })
        XCTAssertEqual(scaled.diameter, stick.diameter * 1.5, accuracy: 0.001)
        XCTAssertEqual(scaled.unitSize, stick.unitSize * 1.5, accuracy: 0.001)
        input.process([
            .init(id: 1, phase: .moved, position: a.center),
            .init(id: 1, phase: .ended, position: a.center),
        ])
        XCTAssertEqual(input.gamepad, .neutral)
    }

    func testDraggedEdgePositionsClampWithLargeControlsAndInvalidValuesFallback() throws {
        let layout = try TouchLayoutCatalog.standard()
        let size = CGSize(width: 390, height: 422)
        let base = TouchLayoutGeometry.sockets(content: layout.content, size: size)
        for point in [CGPoint(x: -10, y: -10), CGPoint(x: 10, y: 10)] {
            let configuration = TouchLayoutConfiguration(
                positions: Dictionary(
                    uniqueKeysWithValues:
                        base.map { ($0.id, TouchPoint(point)) }), scale: 1.5)
            let sockets = TouchLayoutGeometry.configured(
                base, size: size, insets: .init(), configuration: configuration)
            for socket in sockets {
                XCTAssertTrue(CGRect(origin: .zero, size: size).contains(socket.frame))
            }
        }
        let defaultSockets = TouchLayoutGeometry.configured(
            base, size: size, insets: .init(), configuration: .init())
        let invalid = TouchLayoutGeometry.configured(
            base, size: size, insets: .init(),
            configuration: .init(
                positions: ["right/inner/1": TouchPoint(CGPoint(x: Double.nan, y: Double.nan))],
                scale: .nan))
        XCTAssertEqual(invalid, defaultSockets)
    }
}
