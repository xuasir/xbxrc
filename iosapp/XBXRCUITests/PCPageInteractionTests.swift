import UIKit
import XCTest

@MainActor
final class PCPageInteractionTests: XCTestCase {
  private func launch(arguments: [String] = []) -> XCUIApplication {
    continueAfterFailure = false
    let app = XCUIApplication()
    app.launchArguments =
      [
        "--pc-pages-ui-test", "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
      ] + arguments
    app.launch()
    XCTAssertTrue(app.buttons["primary-navigation-hosts"].waitForExistence(timeout: 10))
    return app
  }

  func testRootHeadersKeepBrandTitleAndNavigationHeightAligned() throws {
    let app = launch()
    continueAfterFailure = true
    var referenceHeight: CGFloat?
    var referenceBrandX: CGFloat?
    var referenceTitleX: CGFloat?
    var geometry: [String] = []
    for section in ["hosts", "library", "my", "controller", "settings", "my", "hosts"] {
      app.buttons["primary-navigation-\(section)"].tap()
      let title = app.staticTexts["current-page-title"].firstMatch
      XCTAssertTrue(title.waitForExistence(timeout: 5))
      let navigation = app.navigationBars.firstMatch
      let logo = app.descendants(matching: .any)["page-xbox-logo"].firstMatch
      save(app, "reading-root-\(section)")
      XCTAssertGreaterThan(try titleContrast(app: app, title: title), 30)
      geometry.append(
        "\(section): navigation=\(navigation.frame), logo=\(logo.frame), title=\(title.frame)"
      )
      XCTAssertTrue(logo.exists, "根页应使用一致的Xbox标志和标题位置：\(section)")
      XCTAssertEqual(logo.label, "Xbox")
      XCTAssertGreaterThan(try titleContrast(app: app, title: logo), 30)
      if logo.exists {
        if let referenceBrandX, let referenceTitleX {
          XCTAssertEqual(logo.frame.minX, referenceBrandX, accuracy: 1)
          XCTAssertEqual(title.frame.minX, referenceTitleX, accuracy: 1)
        } else {
          referenceBrandX = logo.frame.minX
          referenceTitleX = title.frame.minX
        }
        XCTAssertEqual(logo.frame.width, 28, accuracy: 1)
        XCTAssertEqual(logo.frame.height, 28, accuracy: 1)
        // 28pt图形在40pt槽位居中，右侧6pt留白加上12pt标题间距。
        XCTAssertEqual(title.frame.minX - logo.frame.maxX, 18, accuracy: 1)
        XCTAssertEqual(title.frame.midY, logo.frame.midY, accuracy: 1)
      }
      if let referenceHeight {
        XCTAssertEqual(navigation.frame.height, referenceHeight, accuracy: 1)
      } else {
        referenceHeight = navigation.frame.height
      }
    }
    let attachment = XCTAttachment(string: geometry.joined(separator: "\n"))
    attachment.name = "reading-root-header-geometry"
    attachment.lifetime = .keepAlways
    add(attachment)
    let navigationButton = app.buttons["primary-navigation-hosts"]
    let capsuleBottomGap = app.frame.maxY - navigationButton.frame.maxY - 8
    XCTAssertEqual(capsuleBottomGap, 24, accuracy: 1)
    let horizontalGap = max(24, (app.frame.width - 560) / 2)
    XCTAssertEqual(navigationButton.frame.minX - app.frame.minX - 8, horizontalGap, accuracy: 1)
    XCTAssertEqual(
      app.frame.maxX - app.buttons["primary-navigation-settings"].frame.maxX - 8,
      horizontalGap, accuracy: 1)
    XCTAssertEqual(app.keyboards.count, 0)
  }

  func testSingleHostKeepsCenteredHeroAndCloudToolbarCoversScrolledContent() throws {
    let app = launch()
    let hero = app.descendants(matching: .any)["host-selected-hero"].firstMatch
    XCTAssertTrue(hero.waitForExistence(timeout: 5))
    XCTAssertFalse(app.scrollViews["host-switcher"].exists)
    XCTAssertEqual(
      app.scrollViews["host-overview-scroll"].frame.midX, app.frame.midX, accuracy: 1)
    XCTAssertTrue(app.buttons["host-start-stream"].isHittable)
    save(app, "reading-single-host")

    app.buttons["primary-navigation-library"].tap()
    waitForPage(app, "云游戏")
    let navigation = app.navigationBars.firstMatch
    XCTAssertTrue(app.staticTexts["current-page-title"].waitForExistence(timeout: 5))
    let logo = app.descendants(matching: .any)["page-xbox-logo"].firstMatch
    let settled = XCTNSPredicateExpectation(
      predicate: NSPredicate { _, _ in
        abs(logo.frame.width - 28) <= 0.5 && abs(logo.frame.height - 28) <= 0.5
      }, object: nil)
    XCTAssertEqual(XCTWaiter.wait(for: [settled], timeout: 5), .completed)
    XCTAssertEqual(logo.frame.width, 28, accuracy: 1)
    XCTAssertEqual(logo.frame.height, 28, accuracy: 1)
    let before = try toolbarBackgroundPixels(app: app, navigation: navigation)
    app.scrollViews.firstMatch.swipeUp()
    app.scrollViews.firstMatch.swipeUp()
    let after = try toolbarBackgroundPixels(app: app, navigation: navigation)
    XCTAssertEqual(before, after, "滚动内容经过标题栏时应保持同一画布背景")
    XCTAssertGreaterThan(
      try titleContrast(app: app, title: app.staticTexts["current-page-title"].firstMatch), 30
    )
    save(app, "reading-cloud-scrolled-header")
  }

  private func titleContrast(app: XCUIApplication, title: XCUIElement) throws -> Int {
    let screenshot = try XCTUnwrap(app.screenshot().image.cgImage)
    let scale = CGFloat(screenshot.width) / app.frame.width
    let sample = CGRect(
      x: (title.frame.minX - app.frame.minX) * scale,
      y: (title.frame.minY - app.frame.minY) * scale,
      width: title.frame.width * scale, height: title.frame.height * scale)
    let crop = try XCTUnwrap(screenshot.cropping(to: sample))
    var pixels = [UInt8](repeating: 0, count: crop.width * crop.height * 4)
    pixels.withUnsafeMutableBytes { buffer in
      let context = CGContext(
        data: buffer.baseAddress, width: crop.width, height: crop.height,
        bitsPerComponent: 8, bytesPerRow: crop.width * 4,
        space: CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
      context?.draw(crop, in: CGRect(x: 0, y: 0, width: crop.width, height: crop.height))
    }
    let values = stride(from: 0, to: pixels.count, by: 4).map { Int(pixels[$0]) }
    return (values.max() ?? 0) - (values.min() ?? 0)
  }

  private func toolbarBackgroundPixels(app: XCUIApplication, navigation: XCUIElement) throws
    -> Data
  {
    let screenshot = try XCTUnwrap(app.screenshot().image.cgImage)
    let scale = CGFloat(screenshot.width) / app.frame.width
    let sample = CGRect(
      x: app.frame.width * 0.55 * scale,
      y: (navigation.frame.maxY - app.frame.minY - 6) * scale,
      width: 8 * scale, height: 3 * scale)
    let crop = try XCTUnwrap(screenshot.cropping(to: sample))
    return try XCTUnwrap(UIImage(cgImage: crop).pngData())
  }

  func testSettingsHintsStayWithControlsAndMenuShortcutsHaveOneEntry() {
    let app = launch()
    app.buttons["primary-navigation-settings"].tap()
    app.buttons["section-general"].tap()
    let login = app.switches["settings-toggle-使用临时登录会话"]
    let hint = app.staticTexts["settings-hint-general-login"]
    reveal(hint, in: app)
    XCTAssertTrue(login.exists)
    XCTAssertGreaterThanOrEqual(login.frame.height, 44)
    XCTAssertGreaterThanOrEqual(hint.frame.minY - login.frame.maxY, -1)
    XCTAssertLessThanOrEqual(hint.frame.minY - login.frame.maxY, 8)
    XCTAssertLessThan(hint.frame.height, 44)
    let previous = login.value as? String
    login.tap()
    XCTAssertNotEqual(login.value as? String, previous)
    save(app, "reading-settings-attached-hint")
    XCTAssertEqual(
      app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH 'settings-help-'"))
        .count, 0)

    let sidebar = app.scrollViews["secondary-sidebar"]
    if sidebar.exists {
      XCTAssertEqual(app.buttons["section-general"].label, "通用")
      app.buttons["section-streaming"].tap()
    } else {
      app.buttons["app-navigation-back"].tap()
      app.buttons["section-streaming"].tap()
    }
    let clarity = app.switches["settings-toggle-防止分辨率下降"]
    reveal(clarity, in: app)
    XCTAssertGreaterThanOrEqual(clarity.frame.height, 44)
    let clarityValue = clarity.value as? String
    clarity.tap()
    XCTAssertNotEqual(clarity.value as? String, clarityValue)
    save(app, "reading-streaming-clarity-toggle")
    reveal(app.switches["settings-toggle-显示音量控制"], in: app)
    XCTAssertFalse(app.buttons["shortcut-record-menu"].exists)
    XCTAssertFalse(app.buttons["shortcut-record-diagnostics"].exists)

    app.buttons["primary-navigation-controller"].tap()
    let menu = app.buttons["section-menu"]
    revealSidebar(menu, in: app)
    reveal(menu, in: app)
    menu.tap()
    waitForPage(app, "菜单操作")
    let record = app.buttons["shortcut-record-menu"]
    reveal(record, in: app)
    XCTAssertTrue(record.isEnabled)
    XCTAssertTrue(app.staticTexts["选择目标"].exists)
    XCTAssertTrue(app.staticTexts["录入组合"].exists)
    XCTAssertTrue(app.staticTexts["松开保存"].exists)
    XCTAssertTrue(
      app.descendants(matching: .any)["controller-menu-touch-group"].firstMatch.exists)
    XCTAssertTrue(app.staticTexts["settings-hint-shortcut-input"].exists)
    save(app, "reading-controller-instructions")
    XCTAssertEqual(app.keyboards.count, 0)
  }

  func testHostSwitcherUpdatesHeroActionsAndInstalledTarget() {
    let app = launch(arguments: ["--host-switcher-ui-test"])
    let hero = app.descendants(matching: .any)["host-selected-hero"].firstMatch
    let switcher = app.scrollViews["host-switcher"]
    let name = app.staticTexts["host-selected-name"]
    let play = app.buttons["host-start-stream"]
    let power = app.buttons["host-power"]
    XCTAssertTrue(hero.waitForExistence(timeout: 5))
    XCTAssertTrue(switcher.exists)
    XCTAssertGreaterThanOrEqual(switcher.frame.minY, hero.frame.maxY)
    XCTAssertEqual(name.label, "我的 Xbox")
    XCTAssertEqual(power.label, "关机 我的 Xbox")
    XCTAssertTrue(play.isEnabled)
    save(app, "host-hero-series-x")

    let standby = app.buttons["host-select-fixture-series-s"]
    if !standby.isHittable { switcher.swipeLeft() }
    standby.tap()
    XCTAssertTrue(standby.isSelected)
    XCTAssertEqual(name.label, "卧室 Xbox")
    XCTAssertTrue(app.staticTexts["Xbox Series S"].exists)
    XCTAssertEqual(play.label, "开始游玩 卧室 Xbox")
    XCTAssertEqual(power.label, "开机 卧室 Xbox")
    XCTAssertTrue(play.isEnabled)
    XCTAssertTrue(power.isEnabled)
    save(app, "host-hero-series-s")

    app.buttons["host-open-installed"].tap()
    waitForPage(app, "安装游戏")
    XCTAssertTrue(app.staticTexts["卧室 Xbox"].exists)
    app.buttons["app-navigation-back"].tap()
    XCTAssertEqual(name.label, "卧室 Xbox")
    XCTAssertTrue(standby.isSelected)

    let offline = app.buttons["host-select-fixture-offline"]
    if !offline.isHittable { switcher.swipeLeft() }
    offline.tap()
    XCTAssertEqual(name.label, "书房 Xbox")
    XCTAssertFalse(play.isEnabled)
    XCTAssertFalse(power.exists)
    save(app, "host-hero-offline")

    switcher.swipeRight()
    app.buttons["host-select-fixture-host"].tap()
    XCTAssertEqual(name.label, "我的 Xbox")
    XCTAssertTrue(play.isEnabled)
    XCTAssertEqual(power.label, "关机 我的 Xbox")
  }

  func testInformationPagesExposeHeroAndKeepLibraryActionsReachable() {
    let app = launch()
    app.buttons["primary-navigation-my"].tap()
    let identity = app.descendants(matching: .any)["personal-identity-hero"].firstMatch
    XCTAssertTrue(identity.waitForExistence(timeout: 5))
    save(app, "information-my-identity")
    app.buttons["primary-navigation-hosts"].tap()
    let installed = app.buttons.matching(identifier: "host-open-installed").firstMatch
    reveal(installed, in: app)
    installed.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    let hero = app.descendants(matching: .any)["installed-host-hero"].firstMatch
    XCTAssertTrue(hero.waitForExistence(timeout: 5))
    XCTAssertTrue(app.staticTexts["存储"].exists)
    XCTAssertTrue(app.buttons["settings-picker-游戏排序"].exists)
    save(app, "information-installed-overview")
    let game = app.buttons["installed-game-1"]
    reveal(game, in: app)
    XCTAssertGreaterThan(game.frame.height, 100)
    save(app, "information-installed-covers")
    XCTAssertEqual(app.keyboards.count, 0)
  }

  func testMySubpagesRestoreRootAndDetailNavigation() {
    let app = launch()
    app.buttons["primary-navigation-settings"].tap()
    app.buttons["section-general"].tap()
    app.buttons["settings-picker-应用外观"].tap()
    app.buttons["暗色"].tap()
    app.buttons["primary-navigation-my"].tap()
    waitForPage(app, "我的")
    XCTAssertFalse(app.scrollViews["my-section-tabs"].exists)
    XCTAssertFalse(app.buttons["app-navigation-back"].exists)
    let refresh = app.buttons["刷新账户资料"]
    XCTAssertTrue(refresh.exists)
    XCTAssertLessThan(refresh.frame.midX, app.frame.maxX)
    XCTAssertGreaterThan(refresh.frame.midX, app.frame.midX)
    let friends = app.buttons["section-friends"]
    reveal(friends, in: app)
    let rootPosition = friends.frame.minY
    save(app, "annotations-my-overview")
    friends.tap()
    waitForPage(app, "好友")
    let friend = app.buttons["friend-2000"]
    reveal(friend, in: app)
    friend.tap()
    waitForPage(app, "在线好友")
    app.buttons["app-navigation-back"].tap()
    waitForPage(app, "好友")
    XCTAssertTrue(friend.exists)
    app.buttons["app-navigation-back"].tap()
    waitForPage(app, "我的")
    XCTAssertEqual(friends.frame.minY, rootPosition, accuracy: 24)
    let signOut = app.buttons["my-sign-out"]
    XCTAssertTrue(signOut.exists)
    XCTAssertGreaterThan(signOut.frame.midX, refresh.frame.midX)
    XCTAssertLessThan(signOut.frame.maxY, app.navigationBars.firstMatch.frame.maxY + 1)
    XCTAssertGreaterThanOrEqual(signOut.frame.width, 44)
    XCTAssertGreaterThanOrEqual(signOut.frame.height, 44)
    signOut.tap()
    let confirmation = app.sheets["退出 Xbox 账户？"]
    XCTAssertTrue(confirmation.waitForExistence(timeout: 3))
    save(app, "annotations-my-sign-out-confirmation")
    if app.buttons["取消"].exists {
      app.buttons["取消"].tap()
    } else {
      let dismiss = app.otherElements["PopoverDismissRegion"]
      XCTAssertTrue(dismiss.exists)
      dismiss.coordinate(withNormalizedOffset: CGVector(dx: 0.02, dy: 0.2)).tap()
    }
    XCTAssertTrue(confirmation.waitForNonExistence(timeout: 3))
    XCTAssertTrue(app.buttons["my-sign-out"].exists)
    save(app, "annotations-my-subpages-return")
    XCTAssertEqual(app.keyboards.count, 0)
  }

  func testMyAlbumSubpageOpensMediaAndRestoresNavigation() {
    let app = launch()
    app.buttons["primary-navigation-my"].tap()
    let album = app.buttons["section-album"]
    reveal(album, in: app)
    album.tap()
    waitForPage(app, "Xbox 相册")
    let media = app.buttons["album-media-screenshot:screenshot"]
    XCTAssertTrue(media.waitForExistence(timeout: 5))
    reveal(media, in: app)
    save(app, "annotations-my-album-page")
    media.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    waitForPage(app, "Halo Infinite")
    XCTAssertTrue(app.buttons["刷新媒体链接"].exists)
    app.buttons["app-navigation-back"].tap()
    waitForPage(app, "Xbox 相册")
    XCTAssertTrue(media.exists)
    app.buttons["app-navigation-back"].tap()
    waitForPage(app, "我的")
    XCTAssertTrue(album.exists)
    XCTAssertEqual(app.keyboards.count, 0)
  }

  func testInstalledUpperRegionAndInlineSearch() {
    let app = launch()
    let installed = app.buttons.matching(identifier: "host-open-installed").firstMatch
    reveal(installed, in: app)
    installed.tap()
    let play = app.buttons["installed-start-stream"]
    XCTAssertTrue(play.waitForExistence(timeout: 5))
    XCTAssertEqual(play.frame.width, 56, accuracy: 1)
    XCTAssertEqual(play.frame.height, 56, accuracy: 1)
    let storage = app.descendants(matching: .any)["installed-storage-summary"].firstMatch
    let search = app.descendants(matching: .any)["installed-content-search"].firstMatch
    XCTAssertTrue(
      app.descendants(matching: .any)["installed-storage-capacity"].firstMatch.exists)
    let artwork = app.images["installed-console-artwork"]
    XCTAssertLessThanOrEqual(storage.frame.width, 320)
    XCTAssertLessThan(storage.frame.maxX, artwork.frame.minX)
    let sort = app.buttons["settings-picker-游戏排序"]
    let device = app.buttons["settings-picker-存储设备"]
    XCTAssertEqual(sort.frame.midY, device.frame.midY, accuracy: 1)
    XCTAssertLessThan(storage.frame.maxY, search.frame.minY)
    XCTAssertGreaterThan(search.frame.minY, app.navigationBars.firstMatch.frame.maxY + 100)
    save(app, "review-installed-regions")
    app.buttons["content-search-toggle"].tap()
    let field = app.textFields["content-search-field"]
    XCTAssertTrue(field.waitForExistence(timeout: 5))
    field.typeText("Halo\n")
    let matching = app.buttons["installed-game-1"]
    reveal(matching, in: app)
    XCTAssertTrue(matching.exists)
    save(app, "review-installed-search-result")
    XCTAssertFalse(app.buttons["installed-game-3"].exists)
    reveal(app.buttons["清除搜索"], in: app)
    app.buttons["清除搜索"].tap()
    reveal(app.buttons["installed-game-3"], in: app)
    XCTAssertTrue(app.buttons["installed-game-3"].exists)
  }

  func testInstalledStorageFilterAndFourSectionDetail() {
    let app = launch()
    app.buttons["primary-navigation-hosts"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
    ).tap()
    let installed = app.buttons.matching(identifier: "host-open-installed").firstMatch
    reveal(installed, in: app)
    installed.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    let sort = app.buttons["settings-picker-游戏排序"]
    XCTAssertTrue(sort.waitForExistence(timeout: 5))
    sort.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    save(app, "priority-installed-sort-menu")
    XCTAssertTrue(app.buttons["占用空间"].isHittable)
    app.buttons["占用空间"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    XCTAssertEqual(sort.value as? String, "占用空间")
    let storage = app.buttons["settings-picker-存储设备"]
    storage.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    app.buttons["扩展存储"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    let game = app.buttons["installed-game-3"]
    reveal(game, in: app)
    XCTAssertFalse(app.buttons["installed-game-1"].exists)
    XCTAssertGreaterThan(game.frame.height, 44)
    game.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    waitForPage(app, "Sea of Thieves")
    let sections = app.scrollViews["personal-game-sections"]
    XCTAssertTrue(sections.exists)
    sections.buttons["personal-game-section-statistics"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
    ).tap()
    XCTAssertTrue(app.staticTexts["游玩统计"].waitForExistence(timeout: 5))
    XCTAssertEqual(app.keyboards.count, 0)
    save(app, "pc-installed-statistics")
    app.buttons["app-navigation-back"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
    ).tap()
    waitForPage(app, "安装游戏")
    let restoredStorage = app.buttons["settings-picker-存储设备"]
    for _ in 0..<8 {
      if restoredStorage.exists { break }
      app.scrollViews["app-information-scroll"].firstMatch.swipeDown()
    }
    XCTAssertEqual(restoredStorage.value as? String, "扩展存储")
    save(app, "pc-installed-filter")
  }

  func testGameOverviewUsesTabsAndCentersEmptyAlbum() {
    let app = launch()
    let installed = app.buttons.matching(identifier: "host-open-installed").firstMatch
    reveal(installed, in: app)
    installed.tap()
    let game = app.buttons["installed-game-2"]
    reveal(game, in: app)
    game.tap()
    waitForPage(app, "Forza Horizon 5")
    let description = app.buttons["game-description-toggle"]
    XCTAssertTrue(description.waitForExistence(timeout: 5))
    XCTAssertEqual(description.label, "展开简介")
    description.tap()
    reveal(description, in: app)
    XCTAssertEqual(description.label, "收起简介")
    save(app, "priority-game-description-expanded")
    description.tap()
    reveal(app.buttons["personal-game-section-album"], in: app)
    XCTAssertFalse(app.buttons["查看成就"].exists)
    XCTAssertFalse(app.buttons["查看我的游戏相册"].exists)
    XCTAssertFalse(app.buttons["查看游玩统计"].exists)
    save(app, "priority-game-overview")
    app.buttons["personal-game-section-album"].tap()
    let empty = app.descendants(matching: .any)["album-empty-state"].firstMatch
    XCTAssertTrue(empty.waitForExistence(timeout: 5))
    let types = app.scrollViews["album-media-types"]
    let scroll = app.scrollViews["app-information-scroll"].firstMatch
    XCTAssertEqual(empty.frame.midX, scroll.frame.midX, accuracy: 2)
    XCTAssertEqual(empty.frame.midY, (types.frame.maxY + scroll.frame.maxY) / 2, accuracy: 24)
    save(app, "priority-game-album-empty")
    let clips = app.buttons["album-media-types-clip"]
    XCTAssertTrue(clips.isHittable)
    clips.tap()
    XCTAssertTrue(clips.isSelected)
    save(app, "priority-game-album-type-bar")
    XCTAssertTrue(empty.exists)
    app.buttons["app-navigation-back"].tap()
    waitForPage(app, "安装游戏")
  }

  func testFriendComparisonAndSecretAchievementReveal() {
    let app = launch()
    app.buttons["primary-navigation-my"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
    ).tap()
    app.buttons["section-friends"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
      .tap()
    let friend = app.buttons["friend-2000"]
    XCTAssertTrue(friend.waitForExistence(timeout: 5))
    reveal(friend, in: app)
    friend.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    waitForPage(app, "在线好友")
    let currentGame = app.buttons["friend-current-game"]
    XCTAssertTrue(currentGame.waitForExistence(timeout: 5))
    let history = app.buttons["friend-game-history"]
    let statistics = app.buttons["friend-game-statistics"]
    XCTAssertEqual(currentGame.frame.midY, history.frame.midY, accuracy: 1)
    XCTAssertEqual(currentGame.frame.midY, statistics.frame.midY, accuracy: 1)
    save(app, "priority-player-actions")
    currentGame.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    waitForPage(app, "Halo Infinite")
    let secret = app.buttons["achievement-core:secret"]
    reveal(secret, in: app)
    XCTAssertTrue(secret.exists)
    XCTAssertFalse(app.staticTexts["探索者"].exists)
    secret.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    let revealButton = app.buttons["achievement-reveal-secret"]
    XCTAssertTrue(revealButton.waitForExistence(timeout: 5))
    revealButton.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    XCTAssertTrue(app.staticTexts["发现隐藏区域"].waitForExistence(timeout: 5))
    let requirement = app.descendants(matching: .any)["achievement-requirement-0"].firstMatch
    XCTAssertTrue(requirement.waitForExistence(timeout: 5))
    XCTAssertEqual(requirement.value as? String, "2 / 10")
    XCTAssertEqual(app.keyboards.count, 0)
    save(app, "pc-friend-achievement-revealed")
  }

  func testCloudFavoritesProductLanguagesAndScreenshotPreview() {
    let app = launch()
    app.buttons["primary-navigation-library"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
    ).tap()
    if app.scrollViews["secondary-sidebar"].exists {
      revealSidebar(app.buttons["section-all"], in: app)
      app.buttons["section-all"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        .tap()
      waitForPage(app, "全部游戏")
    }
    let game = app.buttons.matching(identifier: "cloud-game-HALO00000001").firstMatch
    reveal(game, in: app)
    game.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    waitForPage(app, "Halo Infinite")
    XCTAssertTrue(app.buttons["收藏游戏"].waitForExistence(timeout: 5))
    app.buttons["收藏游戏"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    XCTAssertTrue(app.buttons["移除收藏"].exists)
    let language = app.staticTexts["语言支持"]
    reveal(language, in: app)
    XCTAssertTrue(app.descendants(matching: .any)["cloud-product-fact-界面"].firstMatch.exists)
    XCTAssertTrue(app.descendants(matching: .any)["cloud-product-fact-配音"].firstMatch.exists)
    save(app, "pc-cloud-language-support")
    let screenshot = app.buttons["cloud-screenshot-0"]
    for _ in 0..<8 {
      if screenshot.exists && screenshot.frame.midY > app.navigationBars.firstMatch.frame.maxY
        && screenshot.frame.midY < app.buttons["primary-navigation-settings"].frame.minY
      {
        break
      }
      app.scrollViews["cloud-detail-scroll"].swipeDown()
    }
    XCTAssertTrue(screenshot.exists)
    XCTAssertGreaterThan(screenshot.frame.height, 44)
    screenshot.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    waitForPage(app, "游戏截图")
    XCTAssertTrue(app.buttons["分享截图"].exists)
    XCTAssertTrue(app.staticTexts["正在载入截图"].exists || app.staticTexts["截图加载失败"].exists)
    save(app, "pc-cloud-screenshot-preview")
    app.buttons["cloud-screenshot-close"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
    ).tap()
    waitForPage(app, "Halo Infinite")
    app.buttons["app-navigation-back"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
    ).tap()
    let favorites =
      app.scrollViews["secondary-sidebar"].exists
      ? app.buttons["section-favorites"] : app.buttons["cloud-open-favorites"]
    revealSidebar(favorites, in: app)
    favorites.coordinate(
      withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
    ).tap()
    waitForPage(app, "收藏")
    XCTAssertTrue(app.buttons["cloud-game-HALO00000001"].waitForExistence(timeout: 5))
    XCTAssertEqual(app.keyboards.count, 0)
    save(app, "pc-cloud-favorites")
  }

  func testAppLanguageAppliesToCloudDetailsAndPlaytimeSummary() {
    let app = launch()
    app.buttons["primary-navigation-settings"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
    ).tap()
    app.buttons["section-general"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
      .tap()
    app.buttons["settings-picker-应用语言"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
    ).tap()
    app.buttons["English"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    waitForPage(app, "General")
    if !app.scrollViews["secondary-sidebar"].exists {
      app.buttons["app-navigation-back"].coordinate(
        withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
      ).tap()
    }
    app.buttons["primary-navigation-library"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
    ).tap()
    if app.scrollViews["secondary-sidebar"].exists {
      revealSidebar(app.buttons["section-all"], in: app)
      app.buttons["section-all"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        .tap()
      waitForPage(app, "All games")
    }
    let game = app.buttons.matching(identifier: "cloud-game-HALO00000001").firstMatch
    reveal(game, in: app)
    game.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    waitForPage(app, "Halo Infinite")
    XCTAssertTrue(app.buttons["Add to favorites"].exists)
    let time = app.staticTexts["2.5 hr"]
    reveal(time, in: app)
    XCTAssertTrue(time.exists)
    XCTAssertFalse(app.staticTexts["2.5 小时"].exists)
    save(app, "pc-cloud-english-detail")
    app.buttons["primary-navigation-my"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
    ).tap()
    revealMyEntry(app.buttons["section-playtime"], in: app)
    app.buttons["section-playtime"].tap()
    waitForPage(app, "Playtime statistics")
    let summary = app.descendants(matching: .any)["personal-playtime-summary"].firstMatch
    reveal(summary, in: app)
    XCTAssertTrue(summary.exists)
    XCTAssertEqual(summary.label, "Recorded playtime")
    save(app, "pc-playtime-english-before-value-check")
    XCTAssertEqual(summary.value as? String, "3.5 hr")
    let loaded = expectation(
      for: NSPredicate(format: "value == %@", "3.5 hr"), evaluatedWith: summary)
    XCTAssertEqual(XCTWaiter.wait(for: [loaded], timeout: 5), .completed)
    save(app, "pc-playtime-english-value")
    XCTAssertEqual(app.keyboards.count, 0)
    save(app, "pc-playtime-english-summary")
    revealMyEntry(app.buttons["section-album"], in: app)
    app.buttons["section-album"].tap()
    let duration = app.staticTexts["album-duration-clip:clip"]
    reveal(duration, in: app)
    XCTAssertEqual(duration.label, "30 s")
    let captured = app.staticTexts["album-captured-clip:clip"]
    reveal(captured, in: app)
    XCTAssertTrue(captured.label.contains("Sep"))
    save(app, "pc-album-english-date-duration")
    app.buttons["primary-navigation-hosts"].tap()
    let installed = app.buttons.matching(identifier: "host-open-installed").firstMatch
    reveal(installed, in: app)
    installed.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    let capacity = app.staticTexts.matching(
      NSPredicate(format: "label BEGINSWITH %@", "Available ")
    ).firstMatch
    reveal(capacity, in: app)
    XCTAssertTrue(capacity.exists)
    XCTAssertTrue(app.staticTexts["Storage"].exists)
    save(app, "information-installed-english")
  }

  func testWideSidebarKeepsPrimaryNavigationAndSecondarySelection() throws {
    XCUIDevice.shared.orientation = .landscapeLeft
    defer { XCUIDevice.shared.orientation = .portrait }
    let app = launch()
    guard app.frame.width >= 900 else { throw XCTSkip("宽屏侧栏在 iPad 窗口验证") }
    app.buttons["primary-navigation-my"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
    ).tap()
    revealSidebar(app.buttons["section-friends"], in: app)
    app.buttons["section-friends"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
      .tap()
    waitForPage(app, "好友")
    XCTAssertTrue(app.buttons["app-navigation-back"].exists)
    let friend = app.buttons["friend-2000"]
    reveal(friend, in: app)
    XCTAssertTrue(friend.waitForExistence(timeout: 5))
    save(app, "pc-ipad-friends-tabs")
    app.buttons["primary-navigation-library"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
    ).tap()
    revealSidebar(app.buttons["section-all"], in: app)
    app.buttons["section-all"].coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
      .tap()
    waitForPage(app, "全部游戏")
    XCTAssertTrue(app.scrollViews["secondary-sidebar"].exists)
    save(app, "pc-ipad-cloud-sidebar")
    app.buttons["primary-navigation-my"].coordinate(
      withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)
    ).tap()
    waitForPage(app, "好友")
    XCTAssertEqual(app.staticTexts.matching(identifier: "current-page-title").count, 1)
    XCTAssertEqual(app.keyboards.count, 0)
  }

  func testInformationActionsStayCompactAtWideSizes() throws {
    XCUIDevice.shared.orientation = .landscapeLeft
    defer { XCUIDevice.shared.orientation = .portrait }
    let app = launch()
    guard app.frame.width >= 900 else { throw XCTSkip("宽屏密度在iPad验证") }
    app.buttons["primary-navigation-my"].tap()
    let refresh = app.buttons["刷新账户资料"]
    XCTAssertTrue(refresh.waitForExistence(timeout: 5))
    XCTAssertLessThan(refresh.frame.width, 200)
    XCTAssertLessThanOrEqual(refresh.frame.height, 48)
    save(app, "density-ipad-account")
    app.buttons["section-friends"].tap()
    let friend = app.buttons["friend-2000"]
    reveal(friend, in: app)
    XCTAssertLessThan(friend.frame.height, 88)
    save(app, "density-ipad-friends")
    save(app, "density-ipad-friends-page")
    app.buttons["primary-navigation-library"].tap()
    if app.scrollViews["secondary-sidebar"].exists {
      app.buttons["section-all"].tap()
    }
    let game = app.buttons.matching(identifier: "cloud-game-HALO00000001").firstMatch
    reveal(game, in: app)
    game.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
    let play = app.buttons["cloud-play-action"]
    XCTAssertTrue(play.waitForExistence(timeout: 5))
    XCTAssertGreaterThanOrEqual(play.frame.height, 44)
    XCTAssertLessThanOrEqual(play.frame.height, 48)
    XCTAssertLessThan(play.frame.width, 200)
    save(app, "density-ipad-cloud-play")
  }

  private func revealMyEntry(_ element: XCUIElement, in app: XCUIApplication) {
    for _ in 0..<4 {
      if element.exists { break }
      let back = app.buttons["app-navigation-back"]
      if back.exists { back.tap() }
    }
    reveal(element, in: app)
  }

  private func revealSidebar(_ element: XCUIElement, in app: XCUIApplication) {
    let sidebar = app.scrollViews["secondary-sidebar"]
    guard sidebar.exists else { return }
    for _ in 0..<8 {
      if element.exists
        && element.frame.midY < app.buttons["primary-navigation-settings"].frame.minY
      {
        return
      }
      sidebar.swipeUp()
    }
  }

  private func reveal(_ element: XCUIElement, in app: XCUIApplication) {
    for _ in 0..<10 {
      let top = app.navigationBars.firstMatch.frame.maxY
      let bottom = app.buttons["primary-navigation-settings"].frame.minY
      if element.exists && element.frame.midY > top && element.frame.midY < bottom { return }
      let cloudScroll = app.scrollViews["cloud-detail-scroll"]
      let form = app.scrollViews["app-settings-scroll"].firstMatch
      let my = app.scrollViews["my-information-scroll"].firstMatch
      let information = app.scrollViews["app-information-scroll"].firstMatch
      let library = app.scrollViews["cloud-library-scroll"]
      let browser = app.scrollViews["cloud-browser-scroll"]
      let scroll =
        (cloudScroll.exists
          ? cloudScroll
          : my.exists
            ? my
            : information.exists
              ? information
              : form.exists
                ? form
                : browser.exists
                  ? browser
                  : library.exists ? library : app.scrollViews.firstMatch)
      if element.exists && element.frame.midY <= top {
        scroll.swipeDown()
      } else {
        scroll.swipeUp()
      }
    }
    XCTAssertTrue(element.exists)
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

extension PCPageInteractionTests {
  func testSectionNoticesUseFullWidthAndWholeRowRetries() throws {
    let app = launch(arguments: ["--section-errors-ui-test"])
    let installed = app.buttons["host-open-installed"].firstMatch
    reveal(installed, in: app)
    installed.tap()
    let storage = app.buttons["section-retry-installed-storage"]
    reveal(storage, in: app)
    XCTAssertTrue(storage.waitForExistence(timeout: 5))
    XCTAssertTrue(storage.label.contains("失败"))
    XCTAssertTrue(storage.label.contains("Xbox"))
    XCTAssertTrue(storage.label.contains("重试"))
    XCTAssertGreaterThanOrEqual(storage.frame.height, 44)
    XCTAssertGreaterThan(storage.frame.width, 240)
    let artwork = app.images["installed-console-artwork"]
    XCTAssertTrue(artwork.exists)
    XCTAssertGreaterThanOrEqual(storage.frame.minY, artwork.frame.maxY - 1)
    save(app, "section-errors-installed-storage")
    // 点按说明文字侧，验证整行提示具有恢复行为。
    storage.coordinate(withNormalizedOffset: CGVector(dx: 0.08, dy: 0.2)).tap()
    let recovered = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "exists == false"), object: storage)
    XCTAssertEqual(XCTWaiter.wait(for: [recovered], timeout: 5), .completed)
    XCTAssertTrue(app.staticTexts["installed-storage-capacity"].firstMatch.exists)
    save(app, "section-errors-installed-recovered")

    app.buttons["primary-navigation-my"].tap()
    let friends = app.buttons["section-friends"]
    revealMyEntry(friends, in: app)
    friends.tap()
    let retry = app.buttons["section-retry-friends"]
    reveal(retry, in: app)
    XCTAssertTrue(retry.waitForExistence(timeout: 5))
    XCTAssertTrue(retry.label.contains("好友列表加载失败"))
    XCTAssertGreaterThanOrEqual(retry.frame.height, 44)
    save(app, "section-errors-friends-initial")
    retry.tap()
    XCTAssertTrue(app.buttons["friend-2000"].waitForExistence(timeout: 5))
    save(app, "section-errors-friends-recovered")
  }

  func testCloudSourceFailureKeepsOtherShelvesAndRetriesLocally() throws {
    let app = launch(arguments: ["--section-errors-ui-test"])
    app.buttons["primary-navigation-library"].tap()
    let retry = app.buttons["section-retry-cloud-popular"]
    reveal(retry, in: app)
    XCTAssertTrue(retry.waitForExistence(timeout: 5))
    XCTAssertTrue(retry.label.contains("热门游戏加载失败"))
    XCTAssertGreaterThanOrEqual(retry.frame.height, 44)
    save(app, "section-errors-cloud-source")
    retry.tap()
    let recovered = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "exists == false"), object: retry)
    XCTAssertEqual(XCTWaiter.wait(for: [recovered], timeout: 5), .completed)
    XCTAssertTrue(app.buttons["cloud-game-HALO00000001"].firstMatch.exists)
    save(app, "section-errors-cloud-recovered")
  }
}

extension PCPageInteractionTests {
  func testCloudBrowseCapsulesKeepSearchAndFiltersBound() {
    let app = launch()
    app.buttons["primary-navigation-library"].tap()
    app.buttons["搜索云游戏"].firstMatch.tap()
    waitForPage(app, "搜索")
    let search = app.buttons["content-search-toggle"].firstMatch
    let viewport = app.scrollViews["cloud-browser-scroll"]
    XCTAssertEqual(search.frame.width, min(viewport.frame.width, 1040) - 48, accuracy: 2)
    let touch = app.buttons["cloud-input-filter-true"]
    let sort = app.buttons["settings-picker-排序"]
    XCTAssertEqual(touch.frame.midY, sort.frame.midY, accuracy: 2)
    XCTAssertGreaterThanOrEqual(touch.frame.height, 44)
    touch.tap()
    XCTAssertTrue(touch.isSelected)
    save(app, "calibration-cloud-tool-capsules")
    let tools = app.scrollViews["cloud-browser-tools"].firstMatch
    let category = app.buttons["settings-picker-游戏分类"]
    if !category.isHittable { tools.swipeLeft() }
    category.tap()
    app.buttons["竞速"].firstMatch.tap()
    XCTAssertEqual(category.value as? String, "竞速")
    XCTAssertTrue(app.buttons["cloud-game-FORZA0000001"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["cloud-game-HALO00000001"].exists)
    let reset = app.buttons["重置筛选"]
    if !reset.isHittable { tools.swipeLeft() }
    reset.tap()
    XCTAssertTrue(app.buttons["cloud-game-HALO00000001"].waitForExistence(timeout: 5))
    search.tap()
    let field = app.textFields["content-search-field"].firstMatch
    XCTAssertTrue(field.waitForExistence(timeout: 5))
    field.typeText("Forza\n")
    XCTAssertTrue(app.buttons["cloud-game-FORZA0000001"].exists)
    XCTAssertFalse(app.buttons["cloud-game-HALO00000001"].exists)
    let keyboardDismissed = XCTNSPredicateExpectation(
      predicate: NSPredicate { _, _ in app.keyboards.count == 0 }, object: nil)
    XCTAssertEqual(XCTWaiter.wait(for: [keyboardDismissed], timeout: 5), .completed)
    save(app, "calibration-cloud-search-results")
    app.buttons["清除搜索"].tap()
    XCTAssertTrue(app.buttons["cloud-game-HALO00000001"].waitForExistence(timeout: 5))
  }

  func testInformationFiltersSwitchDirectlyAndKeepSummaryScoped() {
    let app = launch()
    app.buttons["primary-navigation-my"].tap()
    let achievements = app.buttons["section-achievements"]
    revealMyEntry(achievements, in: app)
    achievements.tap()
    waitForPage(app, "成就")
    let completed = app.buttons["history-progress-filter-completed"]
    reveal(completed, in: app)
    XCTAssertTrue(completed.isHittable)
    XCTAssertGreaterThanOrEqual(completed.frame.height, 44)
    XCTAssertFalse(app.staticTexts["已加载游戏"].exists)
    XCTAssertFalse(app.staticTexts["仅统计已载入记录，继续分页可补充数据"].exists)
    let score = app.descendants(matching: .any)["information-metric-成就点数"].firstMatch
    XCTAssertTrue(score.exists)
    XCTAssertEqual(score.value as? String, "120 G")
    let completedCount = app.descendants(matching: .any)["information-metric-全成就游戏"].firstMatch
    XCTAssertEqual(completedCount.value as? String, "1")
    XCTAssertFalse(app.descendants(matching: .any)["information-metric-这些游戏的已解锁成就"].exists)
    let search = app.buttons["content-search-toggle"].firstMatch
    XCTAssertEqual(search.frame.width, app.frame.width - 48, accuracy: 2)
    let sort = app.buttons["settings-picker-排序"].firstMatch
    XCTAssertEqual(sort.frame.midY, completed.frame.midY, accuracy: 2)
    let more = app.buttons["content-load-more"].firstMatch
    reveal(more, in: app)
    XCTAssertEqual(more.frame.midX, app.frame.midX, accuracy: 2)
    XCTAssertGreaterThanOrEqual(more.frame.height, 44)
    app.scrollViews["app-information-scroll"].firstMatch.swipeDown()
    save(app, "calibration-achievements-summary")
    completed.tap()
    XCTAssertTrue(completed.isSelected)
    XCTAssertTrue(app.staticTexts["Forza Horizon 5"].exists)
    XCTAssertFalse(app.staticTexts["Halo Infinite"].exists)
    save(app, "calibration-history-completed-filter")
    app.buttons["history-progress-filter-all"].tap()
    let game = app.staticTexts["Halo Infinite"].firstMatch
    reveal(game, in: app)
    game.tap()
    let unlocked = app.buttons["achievement-state-filter-unlocked"]
    XCTAssertTrue(unlocked.waitForExistence(timeout: 5))
    unlocked.tap()
    XCTAssertTrue(unlocked.isSelected)
    XCTAssertFalse(app.buttons["achievement-core:secret"].exists)
    save(app, "calibration-achievements-unlocked-filter")
    app.buttons["achievement-state-filter-locked"].tap()
    XCTAssertTrue(app.buttons["achievement-core:secret"].waitForExistence(timeout: 5))
    XCTAssertFalse(app.buttons["achievement-core:first"].exists)
    save(app, "calibration-achievements-locked-filter")
  }

  func testControllerInformationAndInputStripKeepToolbarOpaque() throws {
    let app = launch()
    app.buttons["primary-navigation-controller"].tap()
    let connection = app.buttons["section-connection"]
    reveal(connection, in: app)
    connection.tap()
    waitForPage(app, "连接与测试")
    XCTAssertTrue(
      app.descendants(matching: .any)["controller-connection-summary"].firstMatch.exists)
    XCTAssertFalse(app.staticTexts["输入归属"].exists)
    XCTAssertFalse(app.staticTexts["最近十次输入"].exists)
    let navigation = app.navigationBars.firstMatch
    let before = try toolbarEdgePixels(app: app, navigation: navigation)
    let scroll = app.scrollViews["app-settings-scroll"].firstMatch
    for _ in 0..<3 {
      scroll.swipeUp()
      XCTAssertEqual(
        before, try toolbarEdgePixels(app: app, navigation: navigation),
        "正文经过标题区时，顶部及底部整条像素保持连续遮盖")
    }
    let history = app.descendants(matching: .any)["controller-input-history"].firstMatch
    reveal(history, in: app)
    XCTAssertTrue(history.exists)
    XCTAssertGreaterThanOrEqual(history.frame.height, 44)
    save(app, "calibration-controller-input-strip")
    scroll.swipeDown()
    scroll.swipeDown()
    save(app, "calibration-controller-information-card")
  }

  private func toolbarEdgePixels(app: XCUIApplication, navigation: XCUIElement) throws -> Data {
    let screenshot = try XCTUnwrap(app.screenshot().image.cgImage)
    let scale = CGFloat(screenshot.width) / app.frame.width
    var output = Data()
    for y in [navigation.frame.minY, navigation.frame.maxY - 4] {
      let rect = CGRect(
        x: 0, y: (y - app.frame.minY) * scale,
        width: app.frame.width * scale, height: 4 * scale)
      let crop = try XCTUnwrap(screenshot.cropping(to: rect))
      output.append(try XCTUnwrap(UIImage(cgImage: crop).pngData()))
    }
    return output
  }
}
