
// BEGIN TEMPORARY MEDIA HALF PANEL SMOKE
extension PCPageInteractionTests {
  func testMediaHalfPanelSmoke() {
    continueAfterFailure = false
    XCUIDevice.shared.orientation = .portrait
    let app = XCUIApplication()
    app.launchArguments = ["--pc-pages-ui-test", "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN"]
    app.launchEnvironment["ALBUM_SMOKE_BASE"] = "http://127.0.0.1:18764"
    app.launch()
    XCTAssertTrue(app.buttons["primary-navigation-my"].waitForExistence(timeout: 10))
    app.buttons["primary-navigation-my"].tap()
    let album = app.buttons["section-album"]
    reveal(album, in: app)
    album.tap()
    let image = app.buttons["album-media-screenshot:screenshot"]
    XCTAssertTrue(image.waitForExistence(timeout: 5))
    reveal(image, in: app)
    image.tap()
    XCTAssertTrue(app.buttons["媒体操作"].waitForExistence(timeout: 5))
    app.buttons["媒体操作"].tap()
    app.buttons["保存与分享文件"].tap()
    let progress = app.descendants(matching: .any)["album-download-progress"].firstMatch
    XCTAssertTrue(progress.waitForExistence(timeout: 5))
    let advanced = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
      let value = progress.value as? String ?? ""
      return value.contains("%") && value != "0%"
    }, object: nil)
    XCTAssertEqual(XCTWaiter.wait(for: [advanced], timeout: 5), .completed, app.debugDescription)
    let cancel = app.buttons["album-download-cancel"]
    XCTAssertGreaterThan(cancel.frame.minY, app.frame.height * 0.64)
    XCTAssertGreaterThan(cancel.frame.minX, app.frame.width * 0.6)
    save(app, "media-download-half-progress")
    cancel.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.97)))
    XCTAssertTrue(progress.exists)
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.01, dy: 0.35)).press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.35)))
    XCTAssertTrue(progress.exists)
    cancel.tap()
    XCTAssertTrue(app.buttons["媒体操作"].waitForExistence(timeout: 5))
    app.buttons["app-navigation-back"].tap()
    let clip = app.buttons["album-media-clip:clip"]
    XCTAssertTrue(clip.waitForExistence(timeout: 5))
    reveal(clip, in: app)
    clip.tap()
    let player = app.descendants(matching: .any)["album-video-player"].firstMatch
    XCTAssertTrue(player.waitForExistence(timeout: 5))
    XCTAssertLessThan(player.frame.height, app.frame.height * 0.5)
    XCTAssertEqual(player.frame.minX, app.frame.minX, accuracy: 1)
    XCTAssertEqual(player.frame.maxX, app.frame.maxX, accuracy: 1)
    XCTAssertEqual(player.frame.width / player.frame.height, 16.0 / 9.0, accuracy: 0.02)
    let full = app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS[c] %@ OR label CONTAINS[c] %@", "全屏", "Full")).firstMatch
    if !full.exists { player.tap() }
    XCTAssertTrue(full.waitForExistence(timeout: 5), app.debugDescription)
    let pause = app.buttons.matching(NSPredicate(format: "label CONTAINS[c] %@ OR label CONTAINS[c] %@", "暂停", "Pause")).firstMatch
    if pause.exists { pause.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap() }
    save(app, "media-video-inline-theme")
    full.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    let back = app.buttons["app-navigation-back"]
    let entered = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in !back.isHittable }, object: nil)
    XCTAssertEqual(XCTWaiter.wait(for: [entered], timeout: 5), .completed)
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    let close = app.buttons.matching(NSPredicate(format: "identifier CONTAINS[c] 'Done' OR label CONTAINS[c] '完成' OR label CONTAINS[c] '关闭' OR label CONTAINS[c] 'close' OR label CONTAINS[c] 'Done'")).firstMatch
    XCTAssertTrue(close.waitForExistence(timeout: 5), app.debugDescription)
    save(app, "media-video-native-fullscreen")
    close.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    let exited = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in back.isHittable }, object: nil)
    XCTAssertEqual(XCTWaiter.wait(for: [exited], timeout: 5), .completed)
    XCTAssertLessThan(player.frame.height, app.frame.height * 0.5)
  }
}
// END TEMPORARY MEDIA HALF PANEL SMOKE


// BEGIN TEMPORARY STREAM HALF PANEL SMOKE
extension TouchLayoutInteractionTests {
    func testStreamingHalfPanelSmoke() {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["--settings-ui-test", "--streaming-panels-ui-test", "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN"]
        XCUIDevice.shared.orientation = .landscapeLeft
        defer { XCUIDevice.shared.orientation = .portrait }
        app.launch()
        XCTAssertTrue(app.buttons["stream-controls-open"].waitForExistence(timeout: 10))
        app.buttons["stream-controls-open"].tap()
        let safe = streamingSafeRegion(app)
        for id in ["stream-operations-rail", "stream-diagnostics-rail"] {
            let rail = app.descendants(matching: .any)[id].firstMatch
            XCTAssertTrue(rail.waitForExistence(timeout: 5))
            XCTAssertLessThanOrEqual(rail.frame.height, safe.height * 0.5 + 2)
        }
        save(app, "stream-half-rails-landscape")
        app.buttons["stream-picture-settings"].tap()
        let picture = app.descendants(matching: .any)["stream-picture-panel"].firstMatch
        XCTAssertTrue(picture.waitForExistence(timeout: 5))
        XCTAssertLessThanOrEqual(picture.frame.height, app.frame.height * 0.5 + 2)
        XCTAssertTrue(app.buttons["stream-picture-close"].isHittable)
        XCTAssertEqual(picture.frame.maxY, app.frame.maxY, accuracy: 2)
        save(app, "stream-half-picture-landscape")
        app.buttons["stream-picture-close"].tap()
        app.buttons["stream-touch-layout-open"].tap()
        let layout = app.descendants(matching: .any)["stream-touch-layout-panel"].firstMatch
        XCTAssertTrue(layout.waitForExistence(timeout: 5))
        XCTAssertLessThanOrEqual(layout.frame.height, safe.height * 0.5 + 2)
        save(app, "stream-half-layout-landscape")
        app.buttons["stream-touch-layout-close"].tap()
        app.buttons["stream-diagnostics-menu"].tap()
        let diagnostic = app.descendants(matching: .any)["stream-diagnostics-floating-panel"].firstMatch
        XCTAssertTrue(diagnostic.waitForExistence(timeout: 5))
        XCTAssertLessThanOrEqual(diagnostic.frame.height, safe.height * 2 / 3 + 2)
        XCTAssertTrue(app.buttons["stream-diagnostics-close"].isHittable)
        save(app, "stream-half-diagnostics-landscape")
        XCUIDevice.shared.orientation = .portrait
        waitForOrientation(app, landscape: false)
        let portraitSafe = streamingSafeRegion(app)
        XCTAssertLessThanOrEqual(diagnostic.frame.height, portraitSafe.height * 2 / 3 + 2)
        save(app, "stream-half-diagnostics-portrait")
    }
}
// END TEMPORARY STREAM HALF PANEL SMOKE
