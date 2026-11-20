import XCTest

@MainActor
final class TouchLayoutInteractionTests: XCTestCase {
    func testControllerPagesExposeTouchAndExternalGamepadControls() {
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .portrait
        let app = XCUIApplication()
        app.launchArguments = [
            "--settings-ui-test", "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
        ]
        app.launch()
        XCTAssertTrue(app.buttons["primary-navigation-controller"].waitForExistence(timeout: 10))
        app.buttons["primary-navigation-controller"].tap()
        let routes = ["connection", "touch", "sticks", "mapping", "vibration", "menu"]
        for route in routes {
            XCTAssertTrue(app.buttons["section-\(route)"].waitForExistence(timeout: 5))
        }
        XCTAssertFalse(app.buttons["section-keyboard"].exists)
        save(app, "controller-mobile-input-sections")
        for route in ["connection", "touch", "mapping", "menu"] {
            app.buttons["section-\(route)"].tap()
            XCTAssertTrue(app.staticTexts["current-page-title"].waitForExistence(timeout: 5))
            XCTAssertEqual(
                app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "键盘")).count, 0)
            save(app, "controller-mobile-input-\(route)")
            app.buttons["app-navigation-back"].tap()
            XCTAssertTrue(app.buttons["section-\(route)"].waitForExistence(timeout: 5))
        }
    }

    func testInternalLayoutSelectionPressDragAndReset() {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = [
            "--settings-ui-test", "--touch-playground-ui-test", "-AppleLanguages", "(zh-Hans)",
            "-AppleLocale", "zh_CN",
        ]
        app.launch()
        let picker = app.buttons["touch-layout-picker"]
        XCTAssertTrue(picker.waitForExistence(timeout: 5))
        let a = app.buttons["touch-control-right/inner/1"]
        XCTAssertTrue(a.waitForExistence(timeout: 5))
        a.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.staticTexts["touch-input-events"].label.contains("A ended"))
        save(app, "generic-portrait")
        XCUIDevice.shared.orientation = .landscapeLeft
        waitForOrientation(app, landscape: true)
        XCTAssertTrue(a.waitForExistence(timeout: 3))
        a.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.staticTexts["touch-input-events"].label.contains("A ended"))
        save(app, "generic-landscape")
        picker.tap()
        app.buttons["touch-layout-choice-forza:layout-1"].tap()
        let gas = app.buttons["touch-control-right/inner/0"]
        XCTAssertTrue(gas.waitForExistence(timeout: 3))
        gas.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.staticTexts["touch-input-events"].label.contains("RT ended"))
        save(app, "forza-buttons-landscape")
        picker.tap()
        app.buttons["touch-layout-choice-forza:layout-1-throttle-pedal"].tap()
        let start = gas.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let end = gas.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
        start.press(forDuration: 0.1, thenDragTo: end)
        XCTAssertTrue(app.staticTexts["touch-input-events"].label.contains("油门 / 刹车 ended"))
        XCTAssertFalse(app.staticTexts["touch-input-axes"].label.hasSuffix("RT 0.00"))
        save(app, "forza-sticky-landscape")
        let stickyAxes = app.staticTexts["touch-input-axes"].label
        app.buttons["touch-playground-tools"].tap()
        app.buttons["显示记录"].tap()
        XCTAssertTrue(app.staticTexts["touch-input-event-list"].waitForExistence(timeout: 3))
        XCTAssertEqual(app.staticTexts["touch-input-axes"].label, stickyAxes)
        app.buttons["touch-playground-tools"].tap()
        app.buttons["收起记录"].tap()
        app.buttons["touch-playground-tools"].tap()
        app.buttons["采样诊断"].tap()
        XCTAssertTrue(app.navigationBars["采样诊断"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.staticTexts["源移动采样"].exists)
        save(app, "sampling-report-landscape")
        app.buttons["完成"].tap()
        app.buttons["touch-playground-tools"].tap()
        app.buttons["清空输入"].tap()
        XCTAssertTrue(app.staticTexts["touch-input-axes"].label.hasSuffix("RT 0.00"))
        XCUIDevice.shared.orientation = .portrait
        waitForOrientation(app, landscape: false)
        save(app, "forza-pedal-portrait")
    }

    func testCustomTemplateDragSaveSwitchReloadAndVisualAdjustments() {
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .portrait
        let app = XCUIApplication()
        app.launchArguments = [
            "--settings-ui-test", "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
        ]
        app.launch()
        openTouchSettings(app)
        XCTAssertFalse(app.buttons["controller-touch-playground"].exists)
        XCTAssertTrue(app.buttons["controller-touch-adjust"].exists)
        save(app, "touch-template-settings")
        app.buttons["controller-touch-customize"].tap()
        let a = app.buttons["touch-control-right/inner/1"]
        XCTAssertTrue(a.waitForExistence(timeout: 5))
        let before = a.frame
        let start = a.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        let end = start.withOffset(CGVector(dx: -55, dy: -30))
        start.press(forDuration: 0.3, thenDragTo: end)
        XCTAssertLessThan(a.frame.midX, before.midX - 30)
        save(app, "touch-template-dragged")
        app.buttons["touch-template-save"].tap()
        let picker = app.buttons["controller-touch-layout"]
        XCTAssertTrue(picker.waitForExistence(timeout: 5))
        app.buttons["controller-touch-customize"].tap()
        XCTAssertTrue(a.waitForExistence(timeout: 5))
        XCTAssertLessThan(a.frame.midX, before.midX - 30)
        app.buttons["app-navigation-back"].tap()
        picker.tap()
        app.buttons["通用 Xbox"].tap()
        app.buttons["controller-touch-customize"].tap()
        XCTAssertTrue(a.waitForExistence(timeout: 5))
        XCTAssertEqual(a.frame.midX, before.midX, accuracy: 3)
        app.buttons["app-navigation-back"].tap()
        picker.tap()
        app.buttons["自定义布局 1"].tap()
        app.buttons["controller-touch-adjust"].tap()
        XCTAssertTrue(a.waitForExistence(timeout: 5))
        let originalWidth = a.frame.width
        let opacity = app.sliders["touch-opacity"]
        let size = app.sliders["touch-size"]
        XCTAssertTrue(size.waitForExistence(timeout: 5))
        opacity.adjust(toNormalizedSliderPosition: 0.3)
        size.adjust(toNormalizedSliderPosition: 0.9)
        let savedOpacity = app.staticTexts["touch-opacity-value"].label
        let savedSize = app.staticTexts["touch-size-value"].label
        XCTAssertGreaterThan(a.frame.width, originalWidth * 1.2)
        save(app, "touch-template-adjusted-preview")
        app.buttons["touch-appearance-save"].tap()
        app.terminate()
        app.launchArguments.append("--preserve-settings-ui-test")
        app.launch()
        openTouchSettings(app)
        app.buttons["controller-touch-adjust"].tap()
        XCTAssertTrue(size.waitForExistence(timeout: 5))
        XCTAssertEqual(app.staticTexts["touch-opacity-value"].label, savedOpacity)
        XCTAssertEqual(app.staticTexts["touch-size-value"].label, savedSize)
        XCTAssertGreaterThan(a.frame.width, originalWidth * 1.2)
        save(app, "touch-template-restored-after-relaunch")
        size.adjust(toNormalizedSliderPosition: 0)
        app.buttons["app-navigation-back"].tap()
        app.buttons["controller-touch-adjust"].tap()
        XCTAssertTrue(size.waitForExistence(timeout: 5))
        XCTAssertEqual(app.staticTexts["touch-size-value"].label, savedSize)
        app.buttons["touch-appearance-reset"].tap()
        XCTAssertEqual(app.staticTexts["touch-size-value"].label, "100%")
        XCTAssertEqual(app.staticTexts["touch-opacity-value"].label, "65%")
    }

    func testPortraitLayoutEditorKeepsControlsInLowerHalfAndDiscardsDraft() {
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .portrait
        let app = XCUIApplication()
        app.launchArguments = [
            "--settings-ui-test", "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
        ]
        app.launch()
        openTouchSettings(app)
        app.buttons["controller-touch-customize"].tap()
        let directions = app.segmentedControls["touch-preview-orientation"]
        XCTAssertTrue(directions.waitForExistence(timeout: 5))
        directions.buttons["竖屏"].tap()
        app.scrollViews["app-settings-scroll"].swipeUp()
        let preview = app.otherElements["touch-layout-preview"]
        XCTAssertTrue(preview.exists)
        // 预览标识传递给实际 UIKit 触摸面；该范围已经是竖屏画布的下半区。
        let region = preview.frame
        XCTAssertEqual(region.height, region.width * 844 / 390 / 2, accuracy: 3)
        let a = app.buttons["touch-control-right/inner/1"]
        XCTAssertTrue(a.exists)
        for id in ["right/inner/1", "left/outer/1", "upper/right/0", "lower/center"] {
            let control = app.buttons["touch-control-\(id)"]
            XCTAssertGreaterThanOrEqual(control.frame.minY, region.minY - 1, id)
            XCTAssertLessThanOrEqual(control.frame.maxY, region.maxY + 1, id)
        }
        save(app, "touch-template-portrait-lower-half")
        let originalX = a.frame.midX
        let start = a.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 0.3, thenDragTo: start.withOffset(CGVector(dx: -40, dy: -35)))
        XCTAssertLessThan(a.frame.midX, originalX - 20)
        app.buttons["app-navigation-back"].tap()
        app.buttons["controller-touch-customize"].tap()
        XCTAssertTrue(directions.waitForExistence(timeout: 5))
        directions.buttons["竖屏"].tap()
        app.scrollViews["app-settings-scroll"].swipeUp()
        XCTAssertEqual(a.frame.midX, originalX, accuracy: 3)
        app.buttons["touch-template-save"].tap()
        app.buttons["controller-touch-customize"].tap()
        XCTAssertTrue(app.buttons["touch-template-delete"].waitForExistence(timeout: 5))
        app.scrollViews["app-settings-scroll"].swipeUp()
        app.buttons["touch-template-delete"].tap()
        let confirmDelete = app.buttons["touch-template-confirm-delete"].firstMatch
        XCTAssertTrue(confirmDelete.waitForExistence(timeout: 5))
        confirmDelete.tap()
        XCTAssertTrue(app.buttons["controller-touch-layout"].waitForExistence(timeout: 5))
    }

    private func openTouchSettings(_ app: XCUIApplication) {
        XCTAssertTrue(app.buttons["primary-navigation-controller"].waitForExistence(timeout: 10))
        app.buttons["primary-navigation-controller"].tap()
        if app.buttons["section-touch"].exists { app.buttons["section-touch"].tap() }
        XCTAssertTrue(app.buttons["controller-touch-customize"].waitForExistence(timeout: 5))
    }

    func testStreamingDiagnosticMenuGroupsCopyAndMetricsVisibility() {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = [
            "--settings-ui-test", "--streaming-panels-ui-test", "-AppleLanguages", "(zh-Hans)",
            "-AppleLocale", "zh_CN",
        ]
        XCUIDevice.shared.orientation = .landscapeLeft
        defer { XCUIDevice.shared.orientation = .portrait }
        app.launch()
        let reveal = app.buttons["stream-controls-open"]
        XCTAssertTrue(reveal.waitForExistence(timeout: 10))
        XCTAssertTrue(reveal.isHittable)
        XCTAssertGreaterThanOrEqual(reveal.frame.width, 44)
        XCTAssertGreaterThanOrEqual(reveal.frame.height, 44)
        XCTAssertTrue(app.buttons["touch-control-right/inner/1"].exists)
        save(app, "streaming-hidden-landscape")
        reveal.tap()
        let diagnostics = app.buttons["stream-diagnostics-menu"]
        XCTAssertTrue(diagnostics.waitForExistence(timeout: 5))
        let metrics = app.buttons["stream-metrics-toggle"]
        if metrics.label == "隐藏运行指标" { metrics.tap() }
        metrics.tap()
        XCTAssertTrue(app.otherElements["compact-performance-overlay"].waitForExistence(timeout: 3))
        for id in [
            "stream-picture-settings", "stream-audio-toggle", "stream-microphone-toggle",
            "stream-aspect-toggle", "stream-paste-text", "stream-exit", "stream-power-off",
            "stream-record-toggle", "stream-trace-export",
        ] {
            XCTAssertTrue(app.buttons[id].exists, id)
            XCTAssertTrue(app.buttons[id].isHittable, id)
        }
        let window = app.windows.firstMatch.frame
        XCTAssertLessThan(app.buttons["stream-picture-settings"].frame.maxX, window.width * 0.2)
        XCTAssertGreaterThan(diagnostics.frame.minX, window.width * 0.8)
        save(app, "streaming-two-rails-landscape")
        diagnostics.tap()
        let panel = app.otherElements["stream-diagnostics-floating-panel"]
        XCTAssertTrue(panel.waitForExistence(timeout: 5))
        XCTAssertLessThan(panel.frame.width, window.width * 0.7)
        let groups = app.segmentedControls["stream-diagnostic-groups"]
        XCTAssertTrue(groups.waitForExistence(timeout: 5))
        for (title, key) in [
            ("网络", "rtt"), ("显示", "receivedFps"), ("恢复", "recoveryEffect"),
            ("会话", "connectedElapsed"),
        ] {
            groups.buttons[title].tap()
            XCTAssertTrue(
                app.otherElements["stream-diagnostic-\(key)"].exists
                    || app.staticTexts["stream-diagnostic-\(key)"].exists)
        }
        app.scrollViews["stream-diagnostic-rows"].swipeUp()
        app.buttons["stream-diagnostic-copy"].tap()
        XCTAssertTrue(app.staticTexts["已复制"].waitForExistence(timeout: 3))
        save(app, "streaming-diagnostics-upper-right")
        app.buttons["stream-diagnostics-close"].tap()
        XCTAssertTrue(metrics.waitForExistence(timeout: 5))
        metrics.tap()
        XCTAssertFalse(app.otherElements["compact-performance-overlay"].exists)
        app.buttons["stream-picture-settings"].tap()
        XCTAssertTrue(app.buttons["stream-picture-close"].waitForExistence(timeout: 5))
        save(app, "streaming-picture-bottom-sheet")
        app.buttons["stream-picture-close"].tap()
        XCTAssertTrue(app.buttons["stream-controls-close"].waitForExistence(timeout: 5))
        app.buttons["stream-controls-close"].tap()
        XCTAssertFalse(diagnostics.exists)
        XCTAssertTrue(reveal.waitForExistence(timeout: 3))
        reveal.tap()
        XCTAssertTrue(diagnostics.waitForExistence(timeout: 3))
    }

    func testPortraitStreamingControlsStayBelowMidpointAndDiagnosticsRemainPartial() {
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .portrait
        let app = XCUIApplication()
        app.launchArguments = [
            "--settings-ui-test", "--streaming-panels-ui-test", "-AppleLanguages", "(zh-Hans)",
        ]
        app.launch()
        let reveal = app.buttons["stream-controls-open"]
        XCTAssertTrue(reveal.waitForExistence(timeout: 10))
        waitForOrientation(app, landscape: false)
        let window = app.windows.firstMatch.frame
        for id in [
            "right/inner/1", "left/outer/1", "upper/right/0", "upper/right/1", "lower/center",
        ] {
            let button = app.buttons["touch-control-\(id)"]
            XCTAssertTrue(button.exists, id)
            XCTAssertGreaterThanOrEqual(button.frame.minY, window.midY, id)
        }
        reveal.tap()
        XCTAssertTrue(app.buttons["stream-power-off"].waitForExistence(timeout: 5))
        save(app, "streaming-two-rails-portrait")
        app.buttons["stream-exit"].tap()
        XCTAssertTrue(app.alerts["结束串流？"].waitForExistence(timeout: 3))
        app.alerts.buttons["取消"].tap()
        XCTAssertTrue(app.buttons["stream-controls-close"].waitForExistence(timeout: 3))
        app.buttons["stream-diagnostics-menu"].tap()
        let panel = app.otherElements["stream-diagnostics-floating-panel"]
        XCTAssertTrue(panel.waitForExistence(timeout: 5))
        XCTAssertLessThan(panel.frame.height, window.height * 0.6)
        save(app, "streaming-diagnostics-portrait")
        app.buttons["stream-diagnostics-close"].tap()
        XCTAssertTrue(app.buttons["stream-controls-close"].exists)
    }

    func testTwoFingerGestureRevealsControlsAndSingleTapKeepsHidden() throws {
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .landscapeLeft
        defer { XCUIDevice.shared.orientation = .portrait }
        let app = XCUIApplication()
        app.launchArguments = ["--settings-ui-test", "--streaming-panels-ui-test"]
        app.launch()
        XCTAssertTrue(app.buttons["stream-controls-open"].waitForExistence(timeout: 10))
        app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.4)).tap()
        XCTAssertFalse(app.buttons["stream-diagnostics-menu"].exists)
        let size = app.windows.firstMatch.frame.size
        try XCTSkipIf(
            max(size.width, size.height) > 1_000, "手机水平双指手势使用 iPhone 验收；iPad 覆盖双栏布局和局部面板。")
        app.windows.firstMatch.pinch(withScale: 0.2, velocity: -4)
        XCTAssertTrue(app.buttons["stream-diagnostics-menu"].waitForExistence(timeout: 3))
        save(app, "streaming-two-finger-gesture")
    }

    func testSignedOutXBXRCLogoAndBottomLoginButton() {
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .portrait
        let app = XCUIApplication()
        app.launchArguments = [
            "--settings-ui-test", "--signed-out-ui-test", "-AppleLanguages", "(zh-Hans)",
        ]
        app.launch()
        let button = app.buttons["signed-out-login"]
        XCTAssertTrue(button.waitForExistence(timeout: 10))
        let logo = app.images["signed-out-logo"]
        XCTAssertTrue(logo.exists)
        XCTAssertEqual(logo.label, "XBXRC")
        XCTAssertTrue(button.isEnabled)
        XCTAssertFalse(app.images["page-xbox-logo"].exists)
        for section in ["hosts", "library", "my", "controller", "settings"] {
            XCTAssertFalse(app.buttons["primary-navigation-\(section)"].exists)
        }
        let window = app.windows.firstMatch.frame
        XCTAssertEqual(logo.frame.midX, window.midX, accuracy: 2)
        XCTAssertEqual(logo.frame.midY, window.midY, accuracy: 20)
        XCTAssertGreaterThan(button.frame.minY, logo.frame.maxY)
        XCTAssertGreaterThan(button.frame.minY, window.height * 0.75)
        XCTAssertGreaterThanOrEqual(window.maxY - button.frame.maxY, 32)
        XCTAssertLessThan(window.maxY - button.frame.maxY, 100)
        XCTAssertGreaterThan(button.frame.width, min(300, window.width * 0.75))
        XCTAssertGreaterThanOrEqual(button.frame.height, 51.9)
        save(app, "signed-out-xbxrc-logo-bottom-login")
    }

    private func save(_ app: XCUIApplication, _ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func waitForOrientation(_ app: XCUIApplication, landscape: Bool) {
        let predicate = NSPredicate { _, _ in
            MainActor.assumeIsolated {
                let frame = app.windows.firstMatch.frame
                return landscape ? frame.width > frame.height : frame.height > frame.width
            }
        }
        XCTAssertEqual(
            XCTWaiter.wait(for: [expectation(for: predicate, evaluatedWith: app)], timeout: 5),
            .completed)
    }
}
