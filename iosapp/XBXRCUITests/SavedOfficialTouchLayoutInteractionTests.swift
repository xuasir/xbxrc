import UIKit
import XCTest

@MainActor
final class SavedOfficialTouchLayoutInteractionTests: XCTestCase {
  func testCommunityProtocolLayoutRunsAndSaves() {
    continueAfterFailure = false
    XCUIDevice.shared.orientation = .portrait
    let app = XCUIApplication()
    app.launchArguments = [
      "--pc-pages-ui-test", "--official-touch-layouts-ui-test", "--game-touch-healthy-sources",
      "--game-touch-protocol-ui-test", "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
    ]
    app.launch()
    XCTAssertTrue(app.buttons["primary-navigation-library"].waitForExistence(timeout: 10))
    app.buttons["primary-navigation-library"].tap()
    let game = app.buttons.matching(identifier: "cloud-game-HALO00000001").firstMatch
    reveal(game, in: app)
    game.tap()
    let entry = app.buttons["cloud-official-touch-layouts"]
    reveal(entry, in: app)
    entry.tap()
    let id = "community:better-xcloud:4416b1ab07588a3244a5c02ce1fa5dfa64d8df74:default"
    let preview = app.buttons["game-touch-preview-" + id]
    reveal(preview, in: app)
    preview.tap()
    let close = app.buttons["official-touch-preview-close"]
    XCTAssertTrue(close.waitForExistence(timeout: 8))
    XCTAssertTrue(close.label.contains("保存并退出"))
    waitForPreviewOrientation(app, orientation: "portrait")
    let pad = app.buttons["touch-control-right/outer/0/2"]
    XCTAssertTrue(pad.waitForExistence(timeout: 5))
    let start = pad.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
    start.press(
      forDuration: 0.1, thenDragTo: pad.coordinate(withNormalizedOffset: CGVector(dx: 0.7, dy: 0.5))
    )
    XCTAssertEqual(pad.value as? String, "释放")
    save(app, "protocol-community-portrait")
    XCUIDevice.shared.orientation = .landscapeLeft
    waitForPreviewOrientation(app, orientation: "landscape")
    save(app, "protocol-community-landscape")
    XCUIDevice.shared.orientation = .portrait
    waitForPreviewOrientation(app, orientation: "portrait")
    XCTAssertTrue(close.isHittable)
    close.tap()
    XCTAssertTrue(close.waitForNonExistence(timeout: 5))
    reveal(preview, in: app)
    waitSaved(preview)
    XCTAssertTrue(app.buttons["game-touch-save-" + id].label.contains("重新保存"))
    save(app, "protocol-community-saved")
    XCTAssertFalse(app.staticTexts["布局等待适配"].exists)
  }

  func testSecondOfficialLayoutWithTouchpadPreviewsAndSaves() {
    continueAfterFailure = false
    XCUIDevice.shared.orientation = .portrait
    let app = XCUIApplication()
    app.launchArguments = [
      "--pc-pages-ui-test", "--official-touch-layouts-ui-test", "--game-touch-engine-ui-test",
      "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
    ]
    app.launch()
    XCTAssertTrue(app.buttons["primary-navigation-library"].waitForExistence(timeout: 10))
    app.buttons["primary-navigation-library"].tap()
    let game = app.buttons.matching(identifier: "cloud-game-HALO00000001").firstMatch
    reveal(game, in: app)
    game.tap()
    let entry = app.buttons["cloud-official-touch-layouts"]
    reveal(entry, in: app)
    entry.tap()
    let preview = app.buttons["game-touch-preview-fixture-1:alternative"]
    reveal(preview, in: app)
    XCTAssertTrue(preview.label.contains("触摸板操作"))
    let saveButton = app.buttons["game-touch-save-fixture-1:alternative"]
    waitEnabled(saveButton)
    saveButton.tap()
    waitSaved(preview)
    XCTAssertTrue(saveButton.label.contains("重新保存"))
    save(app, "second-layout-card-saved")
    preview.tap()
    let close = app.buttons["official-touch-preview-close"]
    XCTAssertTrue(close.waitForExistence(timeout: 8))
    XCTAssertTrue(close.label.contains("保存并退出"))
    let pad = app.buttons["touch-control-right/inner/1"]
    XCTAssertTrue(pad.waitForExistence(timeout: 5))
    let start = pad.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
    let end = pad.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.25))
    start.press(forDuration: 0.1, thenDragTo: end)
    XCTAssertEqual(pad.value as? String, "释放")
    save(app, "second-layout-touchpad-preview")
    close.tap()
    XCTAssertTrue(preview.waitForExistence(timeout: 5))
    waitSaved(preview)
    XCTAssertFalse(app.staticTexts["布局等待适配"].exists)
    app.buttons["primary-navigation-controller"].tap()
    app.buttons["section-touch"].tap()
    let management = app.buttons["controller-saved-touch-layouts"]
    reveal(management, in: app)
    management.tap()
    let saved = app.descendants(matching: .any)["saved-touch-layout-official:fixture-1:alternative"]
      .firstMatch
    XCTAssertTrue(saved.waitForExistence(timeout: 5))
    save(app, "second-layout-saved-library")
  }

  func testOfficialAndCommunityDynamicStylesSaveThroughGameEntry() {
    continueAfterFailure = false
    XCUIDevice.shared.orientation = .portrait
    let app = XCUIApplication()
    app.launchArguments = [
      "--pc-pages-ui-test", "--official-touch-layouts-ui-test", "--game-touch-healthy-sources",
      "--game-touch-tak-ui-test", "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
    ]
    app.launch()
    let library = app.buttons["primary-navigation-library"]
    XCTAssertTrue(library.waitForExistence(timeout: 10))
    library.tap()
    let game = app.buttons.matching(identifier: "cloud-game-HALO00000001").firstMatch
    reveal(game, in: app)
    game.tap()
    let entry = app.buttons["cloud-official-touch-layouts"]
    reveal(entry, in: app)
    entry.tap()
    for (index, bundleID) in [
      "fixture-1", "community:better-xcloud:4416b1ab07588a3244a5c02ce1fa5dfa64d8df74",
    ].enumerated() {
      let preview = app.buttons["game-touch-preview-" + bundleID + ":default"]
      reveal(preview, in: app)
      let saveButton = app.buttons["game-touch-save-" + bundleID + ":default"]
      waitEnabled(saveButton)
      preview.tap()
      let close = app.buttons["official-touch-preview-close"]
      XCTAssertTrue(close.waitForExistence(timeout: 8))
      let button = app.buttons["touch-control-right/inner/1"]
      XCTAssertTrue(button.waitForExistence(timeout: 5))
      button.press(forDuration: 0.4)
      XCTAssertEqual(button.value as? String, "释放")
      save(app, index == 0 ? "tak-official-preview" : "tak-community-preview")
      close.tap()
      XCTAssertTrue(preview.waitForExistence(timeout: 5))
      waitSaved(preview)
    }
  }

  func testCloudAndInstalledLayoutEntriesDownloadBothHealthySources() {
    continueAfterFailure = false
    XCUIDevice.shared.orientation = .portrait
    let app = XCUIApplication()
    app.launchArguments = [
      "--pc-pages-ui-test", "--official-touch-layouts-ui-test", "--game-touch-healthy-sources",
      "--installed-product-only-ui-test", "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
    ]
    app.launch()
    let library = app.buttons["primary-navigation-library"]
    XCTAssertTrue(library.waitForExistence(timeout: 10))
    library.tap()
    let game = app.buttons.matching(identifier: "cloud-game-HALO00000001").firstMatch
    reveal(game, in: app)
    game.tap()
    let cloudEntry = app.buttons["cloud-official-touch-layouts"]
    reveal(cloudEntry, in: app)
    cloudEntry.tap()
    let bundleIDs = [
      "fixture-1", "community:better-xcloud:4416b1ab07588a3244a5c02ce1fa5dfa64d8df74",
    ]
    for (index, bundleID) in bundleIDs.enumerated() {
      let preview = app.buttons["game-touch-preview-" + bundleID + ":default"]
      reveal(preview, in: app)
      XCTAssertTrue(preview.label.contains(index == 0 ? "官方" : "社区"))
      XCTAssertFalse(app.buttons["section-retry-game-touch-source-official"].exists)
      XCTAssertFalse(app.buttons["section-retry-game-touch-source-community"].exists)
      save(app, index == 0 ? "healthy-cloud-official-download" : "healthy-cloud-community-download")
      preview.tap()
      XCTAssertTrue(app.buttons["official-touch-preview-close"].waitForExistence(timeout: 8))
      app.buttons["official-touch-preview-close"].tap()
      XCTAssertTrue(preview.waitForExistence(timeout: 5))
      waitSaved(preview)
    }
    app.buttons["app-navigation-back"].tap()
    app.buttons["primary-navigation-hosts"].tap()
    let installed = app.buttons.matching(identifier: "host-open-installed").firstMatch
    reveal(installed, in: app)
    installed.tap()
    let installedGame = app.buttons["installed-game-1"]
    reveal(installedGame, in: app)
    installedGame.tap()
    let installedEntry = app.buttons["installed-official-touch-layouts"]
    reveal(installedEntry, in: app)
    installedEntry.tap()
    let refresh = app.buttons["game-touch-catalog-refresh"]
    XCTAssertTrue(refresh.waitForExistence(timeout: 5))
    let first = app.buttons["game-touch-preview-fixture-1:default"]
    reveal(first, in: app)
    refresh.tap()
    for (index, bundleID) in bundleIDs.enumerated() {
      let preview = app.buttons["game-touch-preview-" + bundleID + ":default"]
      reveal(preview, in: app)
      XCTAssertTrue(preview.label.contains(index == 0 ? "官方" : "社区"))
      XCTAssertFalse(app.buttons["section-retry-game-touch-source-official"].exists)
      XCTAssertFalse(app.buttons["section-retry-game-touch-source-community"].exists)
      save(
        app,
        index == 0 ? "healthy-installed-official-download" : "healthy-installed-community-download")
      preview.tap()
      XCTAssertTrue(app.buttons["official-touch-preview-close"].waitForExistence(timeout: 8))
      app.buttons["official-touch-preview-close"].tap()
      waitSaved(preview)
    }
  }

  func testCloudListsCurrentGameAndDownloadsOnSelection() {
    continueAfterFailure = false
    XCUIDevice.shared.orientation = .portrait
    let app = XCUIApplication()
    app.launchArguments = [
      "--settings-ui-test", "--streaming-panels-ui-test", "--cloud-touch-layouts-ui-test",
      "--official-touch-layouts-ui-test", "--game-touch-healthy-sources",
      "--seed-saved-touch-layouts",
      "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
    ]
    app.launch()
    openPicker(app)
    XCTAssertEqual(app.staticTexts["stream-touch-current-game"].label, "Halo Infinite")
    let official = app.buttons["stream-touch-layout-choice-cloud-bundle-fixture-1"]
    XCTAssertTrue(official.waitForExistence(timeout: 8))
    XCTAssertEqual(app.staticTexts["stream-touch-downloaded-bundles"].label, "0")
    XCTAssertFalse(app.buttons["stream-touch-layout-choice-official:fixture-2:default"].exists)
    XCTAssertFalse(
      app.buttons["stream-touch-layout-choice-template-00000000-0000-4000-8000-000000000001"].exists
    )
    save(app, "cloud-current-game-query-only")
    official.tap()
    waitForDock(app)
    waitForSelection(app, id: "cloud:fixture-1:default")
    app.buttons["stream-controls-close"].tap()
    let a = app.buttons["touch-control-right/inner/1"]
    a.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).press(forDuration: 0.3)
    waitForButtons(app, mask: "16")
    XCTAssertEqual(app.staticTexts["stream-touch-downloaded-bundles"].label, "1")
    save(app, "cloud-official-layout-applied")
    openPicker(app)
    let alternative = app.buttons["stream-touch-layout-choice-cloud-layout-fixture-1:alternative"]
    revealInPicker(alternative, in: app)
    alternative.tap()
    waitForDock(app)
    app.buttons["stream-controls-close"].tap()
    XCTAssertEqual(app.staticTexts["stream-touch-downloaded-bundles"].label, "1")
    waitForSelection(app, id: "cloud:fixture-1:alternative")
    openPicker(app)
    let communityID = "community:better-xcloud:4416b1ab07588a3244a5c02ce1fa5dfa64d8df74"
    let community = app.buttons["stream-touch-layout-choice-cloud-bundle-" + communityID]
    revealInPicker(community, in: app)
    community.tap()
    waitForDock(app)
    app.buttons["stream-controls-close"].tap()
    XCTAssertEqual(app.staticTexts["stream-touch-downloaded-bundles"].label, "2")
    waitForSelection(app, id: "cloud:" + communityID + ":default")
    save(app, "cloud-community-layout-applied")
    let quick = app.buttons["stream-touch-quick-switch"]
    quick.tap()
    waitForSelection(app, id: "standard")
    quick.tap()
    waitForSelection(app, id: "cloud:" + communityID + ":default")
    XCUIDevice.shared.orientation = .landscapeLeft
    waitForLandscape(app)
    openPicker(app)
    let communityAlternative = app.buttons[
      "stream-touch-layout-choice-cloud-layout-" + communityID + ":alternative"]
    revealInPicker(communityAlternative, in: app)
    save(app, "cloud-current-game-picker-landscape")
    communityAlternative.tap()
    waitForDock(app)
    app.buttons["stream-controls-close"].tap()
    waitForSelection(app, id: "cloud:" + communityID + ":alternative")
    XCTAssertEqual(app.staticTexts["stream-touch-downloaded-bundles"].label, "2")
    XCUIDevice.shared.orientation = .portrait
  }

  func testInstalledGameDetailsOpenCommunityLayoutsWithSourceFailure() {
    continueAfterFailure = false
    XCUIDevice.shared.orientation = .portrait
    let app = XCUIApplication()
    app.launchArguments = [
      "--pc-pages-ui-test", "--official-touch-layouts-ui-test", "--game-touch-mixed-sources",
      "--installed-product-only-ui-test", "-AppleLanguages", "(zh-Hans)",
      "-AppleLocale", "zh_CN",
    ]
    app.launch()
    XCTAssertTrue(app.buttons["primary-navigation-hosts"].waitForExistence(timeout: 10))
    app.buttons["primary-navigation-hosts"].tap()
    let installed = app.buttons.matching(identifier: "host-open-installed").firstMatch
    reveal(installed, in: app)
    installed.tap()
    for id in ["1", "2"] {
      let game = app.buttons["installed-game-" + id]
      reveal(game, in: app)
      game.tap()
      let entry = app.buttons["installed-official-touch-layouts"]
      reveal(entry, in: app)
      XCTAssertTrue(entry.label.contains("查看游戏布局"))
      XCTAssertGreaterThanOrEqual(entry.frame.height, 44)
      save(app, "installed-touch-entry-" + id)
      entry.tap()
      let retry = app.buttons["section-retry-game-touch-source-official"]
      XCTAssertTrue(retry.waitForExistence(timeout: 5))
      XCTAssertTrue(retry.label.contains("登录状态需要更新"))
      let preview = app.buttons[
        "game-touch-preview-community:better-xcloud:4416b1ab07588a3244a5c02ce1fa5dfa64d8df74:default"
      ]
      reveal(preview, in: app)
      XCTAssertTrue(preview.label.contains("社区"))
      save(app, "installed-touch-catalog-" + id)
      app.buttons["app-navigation-back"].tap()
      XCTAssertTrue(entry.waitForExistence(timeout: 5))
      app.buttons["app-navigation-back"].tap()
    }
  }

  func testCommunityLayoutRemainsAvailableWhenOfficialSourceFails() {
    continueAfterFailure = false
    XCUIDevice.shared.orientation = .portrait
    let app = XCUIApplication()
    app.launchArguments = [
      "--pc-pages-ui-test", "--official-touch-layouts-ui-test", "--game-touch-mixed-sources",
      "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
    ]
    app.launch()
    XCTAssertTrue(app.buttons["primary-navigation-library"].waitForExistence(timeout: 10))
    app.buttons["primary-navigation-library"].tap()
    let game = app.buttons.matching(identifier: "cloud-game-HALO00000001").firstMatch
    reveal(game, in: app)
    game.tap()
    let entry = app.buttons["cloud-official-touch-layouts"]
    reveal(entry, in: app)
    XCTAssertTrue(entry.label.contains("查看游戏布局"))
    entry.tap()
    let retry = app.buttons["section-retry-game-touch-source-official"]
    XCTAssertTrue(retry.waitForExistence(timeout: 10))
    let prefix = "community:better-xcloud:4416b1ab07588a3244a5c02ce1fa5dfa64d8df74:"
    let previewEntry = app.buttons["game-touch-preview-" + prefix + "default"]
    reveal(previewEntry, in: app)
    XCTAssertTrue(previewEntry.label.contains("社区"))
    let second = app.buttons["game-touch-preview-" + prefix + "alternative"]
    XCTAssertTrue(second.waitForExistence(timeout: 5))
    XCTAssertGreaterThan(second.frame.minY, previewEntry.frame.minY)
    let saveButton = app.buttons["game-touch-save-" + prefix + "default"]
    XCTAssertEqual(saveButton.frame.midY, previewEntry.frame.midY, accuracy: 1)
    XCTAssertGreaterThan(saveButton.frame.midX, previewEntry.frame.midX)
    let card = app.descendants(matching: .any)["game-touch-card-" + prefix + "default"].firstMatch
    XCTAssertLessThanOrEqual(card.frame.height, 96)
    XCTAssertGreaterThanOrEqual(saveButton.frame.height, 44)
    save(app, "community-touch-layout-cards")
    previewEntry.tap()
    XCTAssertTrue(
      app.descendants(matching: .any)["official-touch-full-preview"].firstMatch.waitForExistence(
        timeout: 8))
    waitForPreviewOrientation(app, orientation: "portrait")
    save(app, "community-touch-preview-portrait")
    XCUIDevice.shared.orientation = .landscapeLeft
    waitForLandscape(app)
    waitForPreviewOrientation(app, orientation: "landscape")
    save(app, "community-touch-preview-landscape")
    XCUIDevice.shared.orientation = .portrait
    let portrait = XCTNSPredicateExpectation(
      predicate: NSPredicate { _, _ in app.frame.height > app.frame.width }, object: nil)
    XCTAssertEqual(XCTWaiter.wait(for: [portrait], timeout: 5), .completed)
    waitForPreviewOrientation(app, orientation: "portrait")
    let close = app.buttons["official-touch-preview-close"]
    XCTAssertTrue(close.label.contains("保存并退出"))
    close.tap()
    XCTAssertTrue(previewEntry.waitForExistence(timeout: 5))
    waitSaved(previewEntry)
    app.buttons["primary-navigation-controller"].tap()
    app.buttons["section-touch"].tap()
    let management = app.buttons["controller-saved-touch-layouts"]
    reveal(management, in: app)
    management.tap()
    XCTAssertTrue(
      app.descendants(matching: .any)["saved-touch-layout-community:fixture-1:default"].firstMatch
        .waitForExistence(timeout: 5))
    save(app, "community-touch-saved-library")
  }

  func testOfficialLayout404UsesSharedRetryAndRecovers() {
    continueAfterFailure = false
    XCUIDevice.shared.orientation = .portrait
    let app = XCUIApplication()
    app.launchArguments = [
      "--pc-pages-ui-test", "--official-touch-layouts-ui-test", "--official-touch-query-failure",
      "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
    ]
    app.launch()
    let library = app.buttons["primary-navigation-library"]
    XCTAssertTrue(library.waitForExistence(timeout: 10))
    library.tap()
    let game = app.buttons.matching(identifier: "cloud-game-HALO00000001").firstMatch
    reveal(game, in: app)
    game.tap()
    let entry = app.buttons["cloud-official-touch-layouts"]
    reveal(entry, in: app)
    entry.tap()
    let retry = app.buttons["section-retry-official-touch-query"]
    XCTAssertTrue(retry.waitForExistence(timeout: 10))
    XCTAssertTrue(retry.label.contains("游戏触屏布局加载失败"))
    XCTAssertFalse(retry.label.contains("XboxBridgeError"))
    XCTAssertGreaterThanOrEqual(retry.frame.height, 44)
    save(app, "official-touch-shared-retry")
    retry.tap()
    let entryPreview = app.buttons["game-touch-preview-fixture-1:default"]
    reveal(entryPreview, in: app)
    entryPreview.tap()
    XCTAssertTrue(
      app.descendants(matching: .any)["official-touch-full-preview"].firstMatch.waitForExistence(
        timeout: 8))
    save(app, "official-touch-recovered")
  }

  func testCloudDetailRetriesPreviewsSavesMultipleLayoutsAndDeletes() {
    continueAfterFailure = false
    XCUIDevice.shared.orientation = .portrait
    let app = XCUIApplication()
    app.launchArguments = [
      "--pc-pages-ui-test", "--official-touch-layouts-ui-test",
      "--official-touch-query-failure",
      "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
    ]
    app.launch()
    XCTAssertTrue(app.buttons["primary-navigation-library"].waitForExistence(timeout: 10))
    app.buttons["primary-navigation-library"].tap()
    let game = app.buttons.matching(identifier: "cloud-game-HALO00000001").firstMatch
    reveal(game, in: app)
    game.tap()
    let entry = app.buttons["cloud-official-touch-layouts"]
    reveal(entry, in: app)
    save(app, "official-touch-cloud-entry")
    entry.tap()
    let retry = app.buttons["section-retry-official-touch-query"]
    XCTAssertTrue(retry.waitForExistence(timeout: 10))
    save(app, "official-touch-query-failure")
    retry.tap()
    let saveButton = app.buttons["game-touch-save-fixture-1:default"]
    XCTAssertTrue(saveButton.waitForExistence(timeout: 10))
    waitEnabled(saveButton)
    save(app, "official-touch-ready-to-save")
    app.buttons["game-touch-preview-fixture-1:default"].tap()
    XCTAssertTrue(app.buttons["official-touch-preview-close"].waitForExistence(timeout: 5))
    save(app, "official-touch-preview-portrait")
    XCUIDevice.shared.orientation = .landscapeLeft
    waitForLandscape(app)
    save(app, "official-touch-preview-landscape")
    app.buttons["official-touch-preview-close"].tap()
    XCUIDevice.shared.orientation = .portrait
    reveal(saveButton, in: app)
    waitSaved(app.buttons["game-touch-preview-fixture-1:default"])
    save(app, "official-touch-saved-default")
    app.buttons["game-touch-preview-fixture-1:unsupported"].tap()
    app.buttons["touch-workspace-settings"].tap()
    XCTAssertTrue(app.staticTexts["布局等待适配"].waitForExistence(timeout: 5))
    XCTAssertTrue(app.buttons["official-touch-preview-close"].label.contains("退出预览"))
    save(app, "official-touch-unsupported")
    app.buttons["official-touch-preview-close"].tap()
    let alternativeSave = app.buttons["game-touch-save-fixture-1:alternative"]
    waitEnabled(alternativeSave)
    reveal(alternativeSave, in: app)
    alternativeSave.tap()
    waitSaved(app.buttons["game-touch-preview-fixture-1:alternative"])
    app.buttons["primary-navigation-controller"].tap()
    app.buttons["section-touch"].tap()
    let management = app.buttons["controller-saved-touch-layouts"]
    reveal(management, in: app)
    management.tap()
    let first = app.descendants(matching: .any)["saved-touch-layout-official:fixture-1:default"]
      .firstMatch
    XCTAssertTrue(first.waitForExistence(timeout: 5))
    save(app, "official-touch-saved-library")
    let remove = app.buttons["saved-touch-delete-official:fixture-1:alternative"]
    reveal(remove, in: app)
    remove.tap()
    app.buttons.matching(identifier: "saved-touch-confirm-delete").firstMatch.tap()
    let gone = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "exists == false"), object: remove)
    XCTAssertEqual(XCTWaiter.wait(for: [gone], timeout: 5), .completed)
    XCTAssertTrue(first.exists)
    save(app, "official-touch-library-after-delete")
  }

  func testStreamingSwitchesSavedAndCustomLayoutsAndKeepsSafeTouchInput() {
    continueAfterFailure = false
    XCUIDevice.shared.orientation = .portrait
    let app = XCUIApplication()
    app.launchArguments = [
      "--settings-ui-test", "--streaming-panels-ui-test", "--official-touch-layouts-ui-test",
      "--seed-saved-touch-layouts", "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
    ]
    app.launch()
    openPicker(app)
    save(app, "official-touch-stream-picker-portrait")
    let first = app.buttons["stream-touch-layout-choice-official:fixture-1:default"]
    revealInPicker(first, in: app)
    first.tap()
    waitForDock(app)
    app.buttons["stream-controls-close"].tap()
    assertControlsInPortraitLowerHalf(app)
    let a = app.buttons["touch-control-right/inner/1"]
    a.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).press(forDuration: 0.3)
    let pressed = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "label == %@", "16"),
      object: app.staticTexts["stream-touch-last-buttons"])
    XCTAssertEqual(XCTWaiter.wait(for: [pressed], timeout: 4), .completed)
    save(app, "official-touch-stream-saved-portrait")
    let quickSwitch = app.buttons["stream-touch-quick-switch"]
    XCTAssertEqual(app.buttons.matching(identifier: "stream-touch-quick-switch").count, 1)
    XCTAssertTrue(quickSwitch.isHittable)
    quickSwitch.tap()
    waitForSelection(app, id: "standard")
    quickSwitch.tap()
    waitForSelection(app, id: "official:fixture-1:default")
    save(app, "official-touch-stream-quick-switch-portrait")
    openPicker(app)
    let search = app.textFields["stream-touch-layout-search"]
    search.tap()
    search.typeText("Forza")
    let second = app.buttons["stream-touch-layout-choice-official:fixture-2:default"]
    XCTAssertTrue(second.waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["stream-touch-layout-choice-official:fixture-1:default"].exists)
    second.tap()
    waitForDock(app)
    app.buttons["stream-controls-close"].tap()
    XCUIDevice.shared.orientation = .landscapeLeft
    waitForLandscape(app)
    save(app, "official-touch-stream-saved-landscape-left")
    openPicker(app)
    save(app, "official-touch-stream-picker-landscape")
    let template = app.buttons[
      "stream-touch-layout-choice-template-00000000-0000-4000-8000-000000000001"]
    revealInPicker(template, in: app)
    template.tap()
    waitForDock(app)
    app.buttons["stream-controls-close"].tap()
    save(app, "official-touch-stream-custom-landscape")
    quickSwitch.tap()
    waitForSelection(app, id: "standard")
    quickSwitch.tap()
    waitForSelection(app, id: "template:00000000-0000-4000-8000-000000000001")
    save(app, "official-touch-stream-quick-custom-landscape")
    XCUIDevice.shared.orientation = .landscapeRight
    waitForLandscape(app)
    openPicker(app)
    app.buttons["stream-touch-layout-choice-standard"].tap()
    waitForDock(app)
    app.buttons["stream-controls-close"].tap()
    save(app, "official-touch-stream-standard-landscape-right")
    XCUIDevice.shared.orientation = .portrait
  }

  func testCloudOfficialLayoutEmptyStateKeepsRetryAvailable() {
    continueAfterFailure = false
    XCUIDevice.shared.orientation = .portrait
    let app = XCUIApplication()
    app.launchArguments = [
      "--pc-pages-ui-test", "--official-touch-layouts-ui-test", "--official-touch-empty",
    ]
    app.launch()
    XCTAssertTrue(app.buttons["primary-navigation-library"].waitForExistence(timeout: 10))
    app.buttons["primary-navigation-library"].tap()
    let game = app.buttons.matching(identifier: "cloud-game-HALO00000001").firstMatch
    reveal(game, in: app)
    game.tap()
    let entry = app.buttons["cloud-official-touch-layouts"]
    reveal(entry, in: app)
    entry.tap()
    XCTAssertTrue(
      app.descendants(matching: .any)["official-touch-empty"].firstMatch.waitForExistence(
        timeout: 8))
    XCTAssertTrue(app.buttons["重新查询"].isHittable)
    save(app, "official-touch-empty")
  }

  private func openPicker(_ app: XCUIApplication) {
    let open = app.buttons["stream-controls-open"]
    XCTAssertTrue(open.waitForExistence(timeout: 5))
    open.tap()
    XCTAssertTrue(app.buttons["stream-touch-layout-open"].waitForExistence(timeout: 5))
    app.buttons["stream-touch-layout-open"].tap()
    XCTAssertTrue(
      app.descendants(matching: .any)["stream-touch-layout-panel"].firstMatch
        .waitForExistence(timeout: 5))
    XCTAssertTrue(
      app.buttons["stream-touch-layout-choice-standard"].waitForExistence(timeout: 5))
  }

  private func revealInPicker(_ element: XCUIElement, in app: XCUIApplication) {
    let scroll = app.scrollViews["stream-touch-layout-scroll"]
    XCTAssertTrue(scroll.waitForExistence(timeout: 5))
    for _ in 0..<6 where !element.exists || !element.isHittable {
      scroll.swipeUp()
    }
    XCTAssertTrue(element.exists)
    XCTAssertTrue(element.isHittable)
  }

  private func waitForDock(_ app: XCUIApplication) {
    XCTAssertTrue(app.buttons["stream-controls-close"].waitForExistence(timeout: 5))
    XCTAssertFalse(
      app.descendants(matching: .any)["stream-touch-layout-panel"].firstMatch.exists)
  }

  private func waitForSelection(_ app: XCUIApplication, id: String) {
    let selected = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "label == %@", id),
      object: app.staticTexts["stream-touch-selected-id"])
    XCTAssertEqual(XCTWaiter.wait(for: [selected], timeout: 5), .completed)
  }

  private func waitEnabled(_ element: XCUIElement) {
    XCTAssertTrue(element.waitForExistence(timeout: 8))
    let ready = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "enabled == true"), object: element)
    XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 8), .completed)
  }

  private func waitSaved(_ element: XCUIElement) {
    let saved = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "label CONTAINS %@", "已保存"), object: element)
    XCTAssertEqual(XCTWaiter.wait(for: [saved], timeout: 8), .completed)
  }

  private func reveal(_ element: XCUIElement, in app: XCUIApplication) {
    XCTAssertTrue(element.waitForExistence(timeout: 8))
    for _ in 0..<8 where !element.isHittable {
      if app.scrollViews["cloud-detail-scroll"].exists {
        app.scrollViews["cloud-detail-scroll"].swipeUp()
      } else {
        app.swipeUp()
      }
    }
    XCTAssertTrue(element.isHittable)
  }

  private func waitForPreviewOrientation(_ app: XCUIApplication, orientation: String) {
    let surface = app.descendants(matching: .any)["official-touch-full-preview"].firstMatch
    let ready = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "value == %@", orientation), object: surface)
    XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 5), .completed)
    let controls = app.buttons.matching(
      NSPredicate(format: "identifier BEGINSWITH %@", "touch-control-"))
    XCTAssertGreaterThan(controls.count, 0)
    for control in controls.allElementsBoundByIndex {
      XCTAssertTrue(app.frame.contains(control.frame))
    }
  }

  private func waitForLandscape(_ app: XCUIApplication) {
    let landscape = XCTNSPredicateExpectation(
      predicate: NSPredicate { _, _ in app.frame.width > app.frame.height }, object: app)
    XCTAssertEqual(XCTWaiter.wait(for: [landscape], timeout: 5), .completed)
  }

  private func assertControlsInPortraitLowerHalf(_ app: XCUIApplication) {
    let controls = app.buttons.matching(
      NSPredicate(format: "identifier BEGINSWITH %@", "touch-control-"))
    XCTAssertGreaterThan(controls.count, 0)
    for control in controls.allElementsBoundByIndex {
      XCTAssertGreaterThanOrEqual(control.frame.minY, app.frame.height / 2 - 2)
      XCTAssertGreaterThanOrEqual(control.frame.width, 44 - 0.001)
      XCTAssertGreaterThanOrEqual(control.frame.height, 44 - 0.001)
      XCTAssertTrue(app.frame.contains(control.frame))
    }
  }

  private func waitForButtons(_ app: XCUIApplication, mask: String) {
    let pressed = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "label == %@", mask),
      object: app.staticTexts["stream-touch-last-buttons"])
    XCTAssertEqual(XCTWaiter.wait(for: [pressed], timeout: 4), .completed)
  }

  private func save(_ app: XCUIApplication, _ name: String) {
    // 整屏抓图沿设备旋转保留完整画布，避免应用矩形在横屏坐标中裁剪。
    let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
    attachment.name = name
    attachment.lifetime = .keepAlways
    add(attachment)
  }
}
