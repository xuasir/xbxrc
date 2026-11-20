import XCTest

@MainActor
final class SettingsInteractionTests: XCTestCase {
    private func launch() -> XCUIApplication {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = [
            "--settings-ui-test", "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
        ]
        app.launch()
        XCTAssertTrue(app.buttons["section-general"].waitForExistence(timeout: 10))
        return app
    }

    func testTogglePickerAndNavigationRespondAtRowEdges() {
        let app = launch()
        app.buttons["section-general"].tap()
        let appearance = app.buttons["settings-picker-应用外观"]
        XCTAssertTrue(appearance.waitForExistence(timeout: 5))
        appearance.coordinate(withNormalizedOffset: CGVector(dx: 0.02, dy: 0.5)).tap()
        XCTAssertTrue(app.buttons["暗色"].waitForExistence(timeout: 3))
        app.buttons["暗色"].tap()
        let feedback = app.switches["settings-toggle-操作触觉"]
        XCTAssertTrue(feedback.exists)
        let initial = feedback.value as? String
        feedback.coordinate(withNormalizedOffset: CGVector(dx: 0.02, dy: 0.5)).tap()
        XCTAssertNotEqual(feedback.value as? String, initial)
        feedback.coordinate(withNormalizedOffset: CGVector(dx: 0.6, dy: 0.5)).tap()
        XCTAssertEqual(feedback.value as? String, initial)
        let icon = app.buttons["settings-action-应用图标"]
        XCTAssertTrue(icon.exists)
        icon.coordinate(withNormalizedOffset: CGVector(dx: 0.98, dy: 0.5)).tap()
        XCTAssertTrue(app.staticTexts["current-page-title"].waitForExistence(timeout: 3))
        XCTAssertEqual(app.staticTexts["current-page-title"].label, "外观与图标")
        save(app, "settings-entire-row")
    }

    func testBottomNavigationReservesSpaceAndFinalRowIsReachable() {
        let app = launch()
        app.buttons["section-general"].tap()
        let last = app.switches["settings-toggle-串流时保持屏幕常亮"]
        for _ in 0..<8 {
            if last.exists
                && last.frame.maxY < app.buttons["primary-navigation-settings"].frame.minY
            {
                break
            }
            contentScroll(in: app).swipeUp()
        }
        XCTAssertTrue(last.exists)
        XCTAssertGreaterThanOrEqual(last.frame.height, 44)
        let bottom = app.buttons["primary-navigation-settings"]
        XCTAssertLessThanOrEqual(last.frame.maxY, bottom.frame.minY)
        let before = last.value as? String
        last.coordinate(withNormalizedOffset: CGVector(dx: 0.02, dy: 0.5)).tap()
        XCTAssertNotEqual(last.value as? String, before)
        XCTAssertTrue(isWide(app) || app.buttons["返回"].exists)
        save(app, "settings-final-row")
    }

    func testPCResolutionAndAspectChoicesAreAvailable() {
        let app = launch()
        app.buttons["section-streaming"].tap()
        let resolution = app.buttons["settings-picker-主机串流分辨率"]
        XCTAssertTrue(resolution.waitForExistence(timeout: 5))
        resolution.coordinate(withNormalizedOffset: CGVector(dx: 0.98, dy: 0.5)).tap()
        XCTAssertTrue(app.buttons["1440p"].waitForExistence(timeout: 3))
        app.buttons["1440p"].tap()
        XCTAssertEqual(resolution.value as? String, "1440p")
        let ratio = app.buttons["settings-picker-初始画面"]
        if !ratio.isHittable { contentScroll(in: app).swipeUp() }
        ratio.tap()
        XCTAssertTrue(app.buttons["18:9"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.buttons["21:9"].exists)
        app.buttons["21:9"].tap()
        XCTAssertEqual(ratio.value as? String, "21:9")
        XCTAssertEqual(app.keyboards.count, 0)
        save(app, "settings-PC-options")
    }

    func testVibrationPictureAndShortcutRecordingStayInline() {
        let app = launch()
        app.buttons["primary-navigation-controller"].tap()
        XCTAssertTrue(app.buttons["section-vibration"].waitForExistence(timeout: 5))
        let inactiveRow = app.buttons["section-general"]
        let inactiveExpectation = expectation(
            for: NSPredicate(format: "isHittable == false"), evaluatedWith: inactiveRow)
        let hiddenResult = XCTWaiter.wait(for: [inactiveExpectation], timeout: 4)
        if hiddenResult != .completed {
            let tree = XCTAttachment(string: app.debugDescription)
            tree.name = "primary-switch-accessibility-tree"
            tree.lifetime = .keepAlways
            add(tree)
        }
        XCTAssertEqual(hiddenResult, .completed)
        app.buttons["section-vibration"].tap()
        XCTAssertTrue(
            app.staticTexts.matching(identifier: "current-page-title")
                .matching(NSPredicate(format: "label == %@", "震动")).firstMatch.waitForExistence(
                    timeout: 5))
        XCTAssertEqual(app.staticTexts.matching(identifier: "current-page-title").count, 1)
        let trigger = app.buttons["controller-hotspot-vibration-rightTrigger"]
        for _ in 0..<6 {
            if trigger.exists && trigger.frame.minY >= app.navigationBars.firstMatch.frame.maxY
                && trigger.frame.maxY < app.buttons["primary-navigation-controller"].frame.minY
            {
                break
            }
            contentScroll(in: app).swipeUp()
        }
        save(app, "controller-hotspot-before")
        XCTAssertTrue(trigger.exists)
        let artwork = app.descendants(matching: .any)["controller-picture-vibration"].firstMatch
        XCTAssertLessThanOrEqual(artwork.frame.width, 441)
        XCTAssertGreaterThanOrEqual(trigger.frame.width + 0.01, 44)
        XCTAssertGreaterThanOrEqual(trigger.frame.height + 0.01, 44)
        trigger.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        let channel = app.buttons["settings-picker-右扳机"]
        XCTAssertTrue(channel.waitForExistence(timeout: 3))
        for _ in 0..<3 {
            if channel.frame.minY >= app.navigationBars.firstMatch.frame.maxY
                && channel.frame.maxY < app.buttons["primary-navigation-controller"].frame.minY
            {
                break
            }
            contentScroll(in: app).swipeUp()
        }
        channel.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        app.buttons["全强"].tap()
        XCTAssertEqual(channel.value as? String, "全强")
        save(app, "controller-picture-vibration")
        if !isWide(app) {
            app.buttons["app-navigation-back"].tap()
            waitForPage(app, "控制器")
        }
        app.buttons["section-menu"].tap()
        waitForPage(app, "菜单操作")
        let menu = app.buttons["shortcut-record-menu"]
        XCTAssertTrue(menu.waitForExistence(timeout: 5))
        menu.tap()
        let status = app.staticTexts["shortcut-capture-status"]
        for _ in 0..<6 {
            if status.exists { break }
            contentScroll(in: app).swipeUp()
        }
        XCTAssertTrue(status.exists)
        if app.buttons["取消录入"].exists {
            XCTAssertEqual(status.label, "松开当前按键并回中摇杆，再输入按键、组合或方向；释放后自动保存。")
            XCTAssertFalse(menu.isEnabled)
            save(app, "controller-inline-recording")
            app.buttons["取消录入"].tap()
            XCTAssertTrue(menu.isEnabled)
        } else {
            XCTAssertEqual(status.label, "连接手柄后选择录入目标。")
            save(app, "controller-inline-recording")
        }
        XCTAssertEqual(app.staticTexts["current-page-title"].label, "菜单操作")
        XCTAssertEqual(app.keyboards.count, 0)
        app.buttons["primary-navigation-settings"].tap()
        if !isWide(app) { waitForPage(app, "设置") }
        app.buttons["primary-navigation-controller"].tap()
        waitForPage(app, "菜单操作")
        if !isWide(app) {
            app.buttons["app-navigation-back"].tap()
            waitForPage(app, "控制器")
        }
    }

    func testLanguageSelectionAppliesToControllerPagesAndDurationUnits() {
        let app = launch()
        app.buttons["section-general"].tap()
        app.buttons["settings-picker-应用语言"].tap()
        XCTAssertTrue(app.buttons["English"].waitForExistence(timeout: 5))
        app.buttons["English"].tap()
        waitForPage(app, "General")
        if !isWide(app) { app.buttons["app-navigation-back"].tap() }
        app.buttons["primary-navigation-controller"].tap()
        if !isWide(app) { waitForPage(app, "Controller") }
        app.buttons["section-mapping"].tap()
        waitForPage(app, "Button and axis mapping")
        let target = app.buttons["settings-picker-映射目标"]
        for _ in 0..<10 {
            if !target.exists {
                contentScroll(in: app).swipeUp()
            } else if target.frame.minY < app.navigationBars.firstMatch.frame.maxY {
                contentScroll(in: app).swipeDown()
            } else if target.frame.maxY > app.buttons["primary-navigation-controller"].frame.minY {
                contentScroll(in: app).swipeUp()
            } else {
                break
            }
        }
        XCTAssertTrue(target.exists)
        XCTAssertEqual(target.label, "Mapping target")
        save(app, "controller-english-mapping")
        XCTAssertGreaterThanOrEqual(target.frame.minY, app.navigationBars.firstMatch.frame.maxY)
        XCTAssertLessThanOrEqual(
            target.frame.maxY, app.buttons["primary-navigation-controller"].frame.minY)
        target.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.buttons["RB"].waitForExistence(timeout: 3))
        app.buttons["RB"].tap()
        XCTAssertEqual(target.value as? String, "RB")
        if !isWide(app) { app.buttons["app-navigation-back"].tap() }
        app.buttons["section-menu"].tap()
        waitForPage(app, "Menu controls")
        XCTAssertTrue(app.staticTexts["View + Menu"].exists)
        XCTAssertTrue(app.staticTexts["Unbound"].exists)
        XCTAssertFalse(app.sliders["Hold duration"].exists)
        XCTAssertEqual(app.keyboards.count, 0)
        save(app, "controller-english-inline")
        if !isWide(app) {
            app.buttons["app-navigation-back"].tap()
            waitForPage(app, "Controller")
        }
        app.buttons["section-connection"].tap()
        waitForPage(app, "Connection and testing")
        XCTAssertTrue(
            app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "App navigation"))
                .firstMatch.waitForExistence(timeout: 3))
        app.buttons["controller-device-details"].tap()
        XCTAssertTrue(
            app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "Input source"))
                .firstMatch
                .exists)
        save(app, "controller-english-connection-facts")
        let settings = app.buttons["primary-navigation-settings"]
        XCTAssertEqual(settings.label, "Settings")
        settings.tap()
        if !isWide(app) { waitForPage(app, "Settings") }
        app.buttons["section-general"].tap()
        waitForPage(app, "General")
        app.buttons["settings-action-应用图标"].tap()
        waitForPage(app, "Appearance and icon")
        app.buttons["settings-picker-应用外观"].tap()
        XCTAssertTrue(app.buttons["Dark"].waitForExistence(timeout: 3))
        app.buttons["Dark"].tap()
        save(app, "settings-english-appearance")
        app.buttons["app-navigation-back"].tap()
        waitForPage(app, "General")
        if !isWide(app) {
            app.buttons["app-navigation-back"].tap()
            waitForPage(app, "Settings")
        }
        app.buttons["section-diagnostics"].tap()
        waitForPage(app, "Advanced diagnostics")
        XCTAssertTrue(app.staticTexts["Ready"].waitForExistence(timeout: 3))
        XCTAssertEqual(app.keyboards.count, 0)
        save(app, "settings-english-diagnostics")
    }

    func testControllerInputEntriesAndDirectMappingSelection() {
        let app = launch()
        app.buttons["primary-navigation-controller"].tap()
        for route in ["touch", "sticks", "mapping"] {
            XCTAssertTrue(app.buttons["section-\(route)"].exists)
        }
        let mapping = app.buttons["section-mapping"]
        for _ in 0..<5 {
            if mapping.isHittable { break }
            contentScroll(in: app).swipeUp()
        }
        mapping.tap()
        waitForPage(app, "按键与轴映射")
        let target = app.buttons["settings-picker-映射目标"]
        let status = app.staticTexts["mapping-capture-status"]
        let picture = app.descendants(matching: .any)["controller-picture-mapping"].firstMatch
        picture.swipeUp()
        XCTAssertTrue(target.isEnabled)
        XCTAssertEqual(target.value as? String, "A")
        for _ in 0..<5 {
            if target.frame.minY >= app.navigationBars.firstMatch.frame.maxY
                && target.frame.maxY < app.buttons["primary-navigation-controller"].frame.minY
            {
                break
            }
            contentScroll(in: app).swipeUp()
        }
        target.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.5)).tap()
        let option = app.buttons["B"]
        XCTAssertTrue(option.waitForExistence(timeout: 3))
        option.tap()
        XCTAssertEqual(target.value as? String, "B")
        XCTAssertTrue(status.waitForExistence(timeout: 3))
        if app.buttons["取消录入"].exists {
            XCTAssertEqual(status.label, "松开当前按键并回中摇杆，再输入按键、组合或方向；释放后自动保存。")
            XCTAssertFalse(target.isEnabled)
            app.buttons["取消录入"].tap()
            XCTAssertTrue(target.isEnabled)
        } else {
            XCTAssertEqual(status.label, "连接手柄后选择录入目标。")
        }
        XCTAssertFalse(app.staticTexts["A 录入"].exists)
        save(app, "annotations-mapping-direct-selection")
        if !isWide(app) { app.buttons["app-navigation-back"].tap() }
        app.buttons["section-connection"].tap()
        waitForPage(app, "连接与测试")
        XCTAssertTrue(app.descendants(matching: .any)["controller-input-history"].firstMatch.exists)
        save(app, "annotations-connection-history")
        if !isWide(app) { app.buttons["app-navigation-back"].tap() }
        app.buttons["section-menu"].tap()
        waitForPage(app, "菜单操作")
        XCTAssertFalse(app.sliders["长按时间"].exists)
        let row = app.descendants(matching: .any)["shortcut-row-menu"].firstMatch
        XCTAssertTrue(row.exists)
        save(app, "annotations-shortcut-whole-row")
        XCTAssertEqual(app.keyboards.count, 0)
    }

    private func isWide(_ app: XCUIApplication) -> Bool {
        app.scrollViews["secondary-sidebar"].exists
    }

    private func contentScroll(in app: XCUIApplication) -> XCUIElement {
        let form = app.scrollViews["app-settings-scroll"].firstMatch
        return form.exists ? form : app.scrollViews.firstMatch
    }

    private func waitForPage(_ app: XCUIApplication, _ title: String) {
        XCTAssertTrue(
            app.staticTexts.matching(identifier: "current-page-title")
                .matching(NSPredicate(format: "label == %@", title)).firstMatch.waitForExistence(
                    timeout: 5))
    }

    private func save(_ app: XCUIApplication, _ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
