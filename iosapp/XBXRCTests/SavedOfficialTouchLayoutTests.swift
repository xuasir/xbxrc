import GameController
import UIKit
import XCTest

@testable import XBXRC

@MainActor
final class SavedOfficialTouchLayoutTests: XCTestCase {
  func testOfficialAssetIsDecodedAndDrawnWithOriginalPixels() async throws {
    let png = UIGraphicsImageRenderer(size: CGSize(width: 8, height: 8)).pngData { context in
      UIColor.red.setFill()
      context.fill(CGRect(x: 0, y: 0, width: 8, height: 8))
    }
    var snapshot = try Self.snapshot(id: "asset", action: "gamepadA")
    var document = try XCTUnwrap(
      JSONSerialization.jsonObject(with: Data(snapshot.documentJson.utf8)) as? [String: Any])
    var content = document["content"] as! [String: Any]
    content["right"] = [
      "inner": [
        [
          "type": "button", "action": "gamepadA",
          "styles": [
            "default": ["faceImage": ["type": "asset", "value": "fire.png"]]
          ],
        ]
      ]
    ]
    document["content"] = content
    snapshot.documentJson = String(
      decoding: try JSONSerialization.data(withJSONObject: document), as: UTF8.self)
    snapshot.assets = [.init(id: "fire.png", data: png)]
    let prepared = try await PreparedTouchLayout.prepare(snapshot)
    XCTAssertTrue(prepared.layout.report.isSupported)
    let image = try XCTUnwrap(prepared.assets.images["fire.png"])
    let input = TouchInputAccumulator()
    input.load(prepared.layout)
    let surface = TouchLayoutSurfaceView(input: input)
    surface.assets = prepared.assets
    surface.frame = CGRect(x: 0, y: 0, width: 800, height: 420)
    surface.layoutSubviews()
    let socket = try XCTUnwrap(
      input.sockets.first { $0.control.action == .gamepad("gamepadA") })
    let drawing = try XCTUnwrap(surface.layer.sublayers?.first { $0.position == socket.center })
    let artwork = try XCTUnwrap(drawing.sublayers?[3].contents) as AnyObject
    XCTAssertTrue(artwork === image)
    XCTAssertTrue(try XCTUnwrap(drawing.sublayers?[4]).isHidden)
    surface.stop()
  }

  func testMissingImagesAndUnknownActionsBlockSaving() async throws {
    var snapshot = try Self.snapshot(id: "unsupported", action: "mouseLeft")
    snapshot.issues = ["必需资源缺失"]
    let prepared = try await PreparedTouchLayout.prepare(snapshot)
    XCTAssertFalse(prepared.layout.report.isSupported)
    let client = TouchLayoutTestClient(items: [], snapshots: [:])
    let library = SavedTouchLayoutsStore(client: client)
    do {
      try await library.save(
        bundle: TouchLayoutTestBundle(snapshot: snapshot), prepared: prepared)
      XCTFail("能力诊断需要阻止保存")
    } catch {}
    let writes = await client.saveCalls
    XCTAssertEqual(writes, 0)
    snapshot.assets = [.init(id: "broken", data: Data([0, 1, 2]))]
    do {
      _ = try await PreparedTouchLayout.prepare(snapshot)
      XCTFail("损坏图片需要被拒绝")
    } catch {}
  }

  func testNewPointerImmediatelyAfterLayoutChangeFollowsOneNeutralFrame() async throws {
    let (controller, session, library, _) = try await setup()
    let a = try XCTUnwrap(
      controller.input.sockets.first { $0.control.action == .gamepad("gamepadA") })
    controller.input.process([.init(id: 1, phase: .began, position: a.center)])
    XCTAssertEqual(session.outputState(now: 1).buttonMask, 16)
    let applied = await controller.select(
      .saved("second"), library: library, preferences: .init())
    XCTAssertTrue(applied)
    let b = try XCTUnwrap(
      controller.input.sockets.first { $0.control.action == .gamepad("gamepadB") })
    controller.input.process([.init(id: 2, phase: .began, position: b.center)])
    XCTAssertEqual(session.outputState(now: 2), .neutral)
    XCTAssertEqual(session.outputState(now: 3).buttonMask, 32)
    controller.input.process([.init(id: 1, phase: .moved, position: b.center)])
    XCTAssertEqual(session.outputState(now: 4).buttonMask, 32)
    controller.input.process([.init(id: 2, phase: .ended, position: b.center)])
    XCTAssertEqual(session.outputState(now: 5), .neutral)
  }

  func testMenuReleaseAllowsFreshTouchAndKeepsPhysicalNeutralGate() throws {
    let pad = GCController.withExtendedGamepad()
    let session = StreamInputSession(controller: { pad })
    let controller = StreamingTouchLayoutController()
    controller.bind(to: session)
    controller.input.setViewport(size: CGSize(width: 800, height: 420))
    _ = session.outputState(now: 0)
    let a = try XCTUnwrap(
      controller.input.sockets.first { $0.control.action == .gamepad("gamepadA") })
    controller.input.process([.init(id: 1, phase: .began, position: a.center)])
    session.setSuppressed(true)
    XCTAssertEqual(controller.input.gamepad, .neutral)
    session.setSuppressed(false)
    controller.input.process([.init(id: 2, phase: .began, position: a.center)])
    XCTAssertEqual(session.outputState(now: 1), .neutral)
    XCTAssertEqual(session.outputState(now: 2).buttonMask, 16)
    session.setSuppressed(true)
    pad.extendedGamepad?.buttonB.setValue(1)
    session.setSuppressed(false)
    controller.input.process([.init(id: 3, phase: .began, position: a.center)])
    XCTAssertEqual(session.outputState(now: 3), .neutral)
    XCTAssertEqual(session.outputState(now: 4), .neutral)
    pad.extendedGamepad?.buttonB.setValue(0)
    XCTAssertEqual(session.outputState(now: 5).buttonMask, 16)
  }

  func testLatestSelectionWinsAfterDelayedLocalRead() async throws {
    let (controller, _, library, client) = try await setup()
    await client.blockLoad("first")
    let old = Task {
      await controller.select(.saved("first"), library: library, preferences: .init())
    }
    await client.waitForLoad("first")
    let latest = await controller.select(
      .saved("second"), library: library, preferences: .init())
    XCTAssertTrue(latest)
    await client.releaseLoad("first")
    let stale = await old.value
    XCTAssertFalse(stale)
    XCTAssertEqual(controller.selection, .saved("second"))
    XCTAssertEqual(controller.input.layout?.id, "second")
    XCTAssertNil(controller.pendingSelection)
  }

  func testSingleQuickSwitchCyclesRecentAndStandardAndCancelsPendingLoad() async throws {
    let (controller, _, library, client) = try await setup()
    XCTAssertNil(controller.nextQuickSelection(preferences: .init(), library: library))
    let saved = await controller.select(.saved("first"), library: library, preferences: .init())
    XCTAssertTrue(saved)
    for _ in 0..<2 {
      XCTAssertEqual(
        controller.nextQuickSelection(preferences: .init(), library: library), .standard)
      let standard = await controller.select(
        .standard, library: library, preferences: .init())
      XCTAssertTrue(standard)
      let recent = try XCTUnwrap(
        controller.nextQuickSelection(preferences: .init(), library: library))
      XCTAssertEqual(recent, .saved("first"))
      let restored = await controller.select(recent, library: library, preferences: .init())
      XCTAssertTrue(restored)
    }
    let standard = await controller.select(.standard, library: library, preferences: .init())
    XCTAssertTrue(standard)
    await client.blockLoad("first")
    let pending = Task {
      await controller.select(.saved("first"), library: library, preferences: .init())
    }
    await client.waitForLoad("first")
    let next = try XCTUnwrap(
      controller.nextQuickSelection(preferences: .init(), library: library))
    XCTAssertEqual(next, .standard)
    pending.cancel()
    let latest = await controller.select(next, library: library, preferences: .init())
    XCTAssertTrue(latest)
    await client.releaseLoad("first")
    let stale = await pending.value
    XCTAssertFalse(stale)
    XCTAssertEqual(controller.selection, .standard)
    XCTAssertNil(controller.pendingSelection)
    XCTAssertEqual(
      controller.nextQuickSelection(preferences: .init(), library: library), .saved("first"))
  }

  func testStoppedSessionAndClosedPickerRejectPendingResults() async throws {
    let (controller, session, library, client) = try await setup()
    await client.blockLoad("first")
    let old = Task {
      await controller.select(.saved("first"), library: library, preferences: .init())
    }
    await client.waitForLoad("first")
    controller.endSession()
    controller.beginSession(generation: 2)
    await client.releaseLoad("first")
    let stale = await old.value
    XCTAssertFalse(stale)
    XCTAssertEqual(controller.selection, .preferences)
    XCTAssertEqual(session.outputState(now: 1), .neutral)
    await client.blockLoad("second")
    let closed = Task {
      await controller.select(.saved("second"), library: library, preferences: .init())
    }
    await client.waitForLoad("second")
    controller.cancelPendingSelection()
    await client.releaseLoad("second")
    let closedResult = await closed.value
    XCTAssertFalse(closedResult)
    XCTAssertEqual(controller.selection, .preferences)
  }

  func testDeletionReplacesCurrentLayoutBeforeResourceRemoval() async throws {
    let (controller, session, library, client) = try await setup()
    let selected = await controller.select(
      .saved("first"), library: library, preferences: .init())
    XCTAssertTrue(selected)
    _ = session.outputState(now: 0)
    let a = try XCTUnwrap(
      controller.input.sockets.first { $0.control.action == .gamepad("gamepadA") })
    controller.input.process([.init(id: 4, phase: .began, position: a.center)])
    XCTAssertEqual(session.outputState(now: 1).buttonMask, 16)
    let item = try XCTUnwrap(library.layouts.first { $0.id == "first" })
    try await library.delete(item, controller: controller)
    XCTAssertEqual(controller.selection, .standard)
    XCTAssertEqual(controller.input.gamepad, .neutral)
    XCTAssertEqual(session.outputState(now: 2), .neutral)
    XCTAssertFalse(library.isAvailable("first"))
    let remaining = try await client.savedLayouts()
    XCTAssertEqual(remaining.map(\.id), ["second"])
  }

  func testToggleStickyAxesAndLayerContributionsReleaseOnReplacement() async throws {
    let (controller, session, library, _) = try await setup()
    let data = Data(
      #"{"content":{"left":{"inner":[{"type":"joystick","axis":{"input":"axisXY","output":"leftJoystick"}}],"outer":[{"type":"button","action":{"type":"layer","target":"extra"}}]},"right":{"inner":[{"type":"button","action":"gamepadA","toggle":true},{"type":"throttle","sticky":true,"axisUp":"rightTrigger","axisDown":"leftTrigger"}]},"layers":{"extra":{"upper":{"right":[{"type":"button","action":"gamepadY"}]}}}}}"#
        .utf8)
    let layout = try XCTUnwrap(TouchLayoutNormalizer.decode(data).first)
    XCTAssertTrue(layout.report.isSupported)
    controller.input.load(layout)
    let toggle = try XCTUnwrap(controller.input.sockets.first { $0.control.toggle })
    let throttle = try XCTUnwrap(controller.input.sockets.first { $0.control.sticky })
    let stick = try XCTUnwrap(controller.input.sockets.first { $0.control.kind == .joystick })
    let layer = try XCTUnwrap(
      controller.input.sockets.first { $0.control.action == .layer("extra") })
    controller.input.process([.init(id: 1, phase: .began, position: toggle.center)])
    controller.input.process([.init(id: 1, phase: .ended, position: toggle.center)])
    controller.input.process([.init(id: 2, phase: .began, position: throttle.center)])
    let pull = CGPoint(x: throttle.center.x, y: throttle.center.y - throttle.diameter * 0.4)
    controller.input.process([.init(id: 2, phase: .moved, position: pull)])
    controller.input.process([.init(id: 2, phase: .ended, position: pull)])
    controller.input.process([.init(id: 3, phase: .began, position: stick.center)])
    controller.input.process([
      .init(
        id: 3, phase: .moved,
        position: CGPoint(x: stick.center.x + stick.diameter * 0.35, y: stick.center.y))
    ])
    controller.input.process([.init(id: 4, phase: .began, position: layer.center)])
    XCTAssertEqual(controller.input.activeLayers, ["extra"])
    XCTAssertNotEqual(controller.input.gamepad, .neutral)
    let applied = await controller.select(.standard, library: library, preferences: .init())
    XCTAssertTrue(applied)
    XCTAssertEqual(controller.input.gamepad, .neutral)
    XCTAssertTrue(controller.input.activeLayers.isEmpty)
    XCTAssertTrue(controller.input.runtimes.values.allSatisfy { !$0.isActive })
    XCTAssertEqual(session.outputState(now: 2), .neutral)
  }

  func testFailedLoadPreservesCurrentLayoutAndTemplateAppearance() async throws {
    let (controller, _, library, client) = try await setup()
    var preferences = MobilePreferences()
    let template = TouchLayoutTemplate(name: "自定义", appearance: .init(opacity: 0.4, scale: 1.2))
    preferences.touchLayoutTemplates = [template]
    let applied = await controller.select(
      .template(template.id), library: library, preferences: preferences)
    XCTAssertTrue(applied)
    await client.removeSnapshot("first")
    let failed = await controller.select(
      .saved("first"), library: library, preferences: preferences)
    XCTAssertFalse(failed)
    XCTAssertEqual(controller.selection, .template(template.id))
    XCTAssertEqual(controller.opacity(preferences: preferences), 0.4)
    XCTAssertEqual(
      controller.configuration(preferences: preferences, orientation: .portrait).scale, 1.2)
    XCTAssertNotNil(controller.loadError)
  }

  func testUpdatedResourceRevisionRejectsPreviouslyPreparedRead() async throws {
    let (controller, _, library, client) = try await setup()
    await client.blockLoad("first")
    let old = Task {
      await controller.select(.saved("first"), library: library, preferences: .init())
    }
    await client.waitForLoad("first")
    await client.updateRevision("first")
    await library.reload()
    await client.releaseLoad("first")
    let result = await old.value
    XCTAssertFalse(result)
    XCTAssertEqual(controller.selection, .preferences)
  }

  func testRecentCustomLayoutSurvivesStandardSwitchAndPreferencesRestart() async throws {
    let (_, _, library, _) = try await setup()
    let suite = "XBXRC.SavedTouchRecent.\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let settings = AppSettingsStore(defaults: defaults)
    let template = TouchLayoutTemplate(
      name: "最近自定义", appearance: .init(opacity: 0.4, scale: 1.2))
    settings.mobile.touchLayoutTemplates = [template]
    let store = StreamingFeatureStore(settingsProvider: settings)
    store.touchLayout.beginSession(generation: 1)
    let custom = await store.touchLayout.select(
      .template(template.id), library: library, preferences: settings.mobile)
    XCTAssertTrue(custom)
    let standard = await store.touchLayout.select(
      .standard, library: library, preferences: settings.mobile)
    XCTAssertTrue(standard)
    XCTAssertEqual(
      store.touchLayout.mostRecentCustomSelection(
        preferences: settings.mobile, library: library), .template(template.id))
    let restoredSettings = AppSettingsStore(defaults: defaults)
    let restored = StreamingFeatureStore(settingsProvider: restoredSettings)
    restored.touchLayout.beginSession(generation: 2)
    let recent = try XCTUnwrap(
      restored.touchLayout.mostRecentCustomSelection(
        preferences: restoredSettings.mobile, library: library))
    let reapplied = await restored.touchLayout.select(
      recent, library: library, preferences: restoredSettings.mobile)
    XCTAssertTrue(reapplied)
    XCTAssertEqual(
      restored.touchLayout.configuration(
        preferences: restoredSettings.mobile, orientation: .portrait
      ).scale, 1.2)
    XCTAssertEqual(restoredSettings.mobile.touchLayoutID, TouchLayoutTemplate.standardID)
    restoredSettings.mobile.deleteTouchTemplate(template.id)
    restored.touchLayout.reconcile(preferences: restoredSettings.mobile, library: library)
    XCTAssertNil(restoredSettings.mobile.recentCustomTouchLayoutID)
    XCTAssertEqual(restored.touchLayout.selection, .standard)
  }

  func testDeletingRecentSavedLayoutClearsPersistentShortcut() async throws {
    var item = Self.item("first")
    item.id = "official:" + String(repeating: "a", count: 64)
    let client = TouchLayoutTestClient(
      items: [item], snapshots: [item.id: try Self.snapshot(id: item.id, action: "gamepadA")])
    let library = SavedTouchLayoutsStore(client: client)
    await library.reload()
    let suite = "XBXRC.SavedTouchRecentDelete.\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let settings = AppSettingsStore(defaults: defaults)
    let store = StreamingFeatureStore(settingsProvider: settings)
    store.touchLayout.beginSession(generation: 1)
    let saved = await store.touchLayout.select(
      .saved(item.id), library: library, preferences: settings.mobile)
    XCTAssertTrue(saved)
    XCTAssertEqual(settings.mobile.recentCustomTouchLayoutID, item.id)
    let restored = StreamingFeatureStore(settingsProvider: AppSettingsStore(defaults: defaults))
    XCTAssertEqual(restored.touchLayout.recentCustomSelection, .saved(item.id))
    try await library.delete(item, controller: store.touchLayout)
    XCTAssertNil(settings.mobile.recentCustomTouchLayoutID)
    XCTAssertNil(store.touchLayout.recentCustomSelection)
    XCTAssertEqual(store.touchLayout.selection, .standard)
  }

  func testSavedRecentSurvivesInitialLibraryRead() async throws {
    var item = Self.item("first")
    item.id = "official:" + String(repeating: "b", count: 64)
    let library = SavedTouchLayoutsStore(
      client: TouchLayoutTestClient(items: [item], snapshots: [:]))
    var preferences = MobilePreferences()
    preferences.recentCustomTouchLayoutID = item.id
    let controller = StreamingTouchLayoutController()
    controller.restoreRecentSelection(preferences: preferences)
    controller.reconcile(preferences: preferences, library: library)
    XCTAssertFalse(library.hasLoadedLayouts)
    XCTAssertEqual(controller.recentCustomSelection, .saved(item.id))
    await library.reload()
    controller.reconcile(preferences: preferences, library: library)
    XCTAssertTrue(library.hasLoadedLayouts)
    XCTAssertEqual(controller.recentCustomSelection, .saved(item.id))
  }

  func testInitialReadFailurePreservesRecentAndSuccessfulEmptyReadClearsIt() async throws {
    let client = TouchLayoutTestClient(items: [], snapshots: [:])
    await client.setListingFailure(true)
    let library = SavedTouchLayoutsStore(client: client)
    var preferences = MobilePreferences()
    preferences.recentCustomTouchLayoutID = "official:" + String(repeating: "c", count: 64)
    let controller = StreamingTouchLayoutController()
    controller.restoreRecentSelection(preferences: preferences)
    var didClearRecent = false
    controller.onRecentSelectionChanged = { id in didClearRecent = id == nil }
    await library.reload()
    controller.reconcile(preferences: preferences, library: library)
    XCTAssertFalse(library.hasLoadedLayouts)
    XCTAssertNotNil(library.errorMessage)
    XCTAssertNotNil(controller.recentCustomSelection)
    XCTAssertFalse(didClearRecent)
    await client.setListingFailure(false)
    await library.reload()
    controller.reconcile(preferences: preferences, library: library)
    XCTAssertTrue(library.hasLoadedLayouts)
    XCTAssertNil(library.errorMessage)
    XCTAssertNil(controller.recentCustomSelection)
    XCTAssertTrue(didClearRecent)
  }

  private func setup() async throws -> (
    StreamingTouchLayoutController, StreamInputSession, SavedTouchLayoutsStore,
    TouchLayoutTestClient
  ) {
    let client = TouchLayoutTestClient(
      items: [Self.item("first"), Self.item("second")],
      snapshots: [
        "first": try Self.snapshot(id: "first", action: "gamepadA"),
        "second": try Self.snapshot(id: "second", action: "gamepadB"),
      ])
    let library = SavedTouchLayoutsStore(client: client)
    await library.reload()
    let controller = StreamingTouchLayoutController()
    let session = StreamInputSession(controller: { nil })
    controller.bind(to: session)
    controller.beginSession(generation: 1)
    controller.input.setViewport(size: CGSize(width: 800, height: 420))
    _ = session.outputState(now: 0)
    return (controller, session, library, client)
  }

  nonisolated private static func snapshot(id: String, action: String) throws
    -> XboxTouchLayoutSnapshot
  {
    let document: [String: Any] = [
      "$schema":
        "https://raw.githubusercontent.com/microsoft/xbox-game-streaming-tools/main/touch-adaptation-kit/schemas/layout/4.1/layout.json",
      "content": ["right": ["inner": [["type": "button", "action": action]]]],
    ]
    return .init(
      id: id, name: id,
      documentJson: String(
        decoding: try JSONSerialization.data(withJSONObject: document), as: UTF8.self),
      assets: [], issues: [])
  }

  nonisolated private static func item(_ id: String) -> XboxSavedTouchLayout {
    .init(
      id: id, productId: "GAME0001", xboxTitleId: "1", gameName: "测试游戏", bundleId: "bundle",
      bundleVersion: "1.0.0.0", layoutId: id + "-layout", layoutName: id,
      isDefault: id == "first", contentHash: "hash", revision: "revision-1", savedAtMs: 1,
      lastUsedAtMs: 0, byteCount: 100)
  }
}

private struct TouchLayoutTestBundle: OfficialTouchBundleProviding {
  let snapshot: XboxTouchLayoutSnapshot
  var version: String { "1.0.0.0" }
  var choices: [XboxTouchLayoutChoice] {
    [.init(id: snapshot.id, name: snapshot.name, isDefault: true, issues: snapshot.issues)]
  }
  func preview(layoutID: String) async throws -> XboxTouchLayoutSnapshot { snapshot }
}

private actor TouchLayoutTestClient: XboxTouchLayoutClient {
  private var items: [XboxSavedTouchLayout]
  private var snapshots: [String: XboxTouchLayoutSnapshot]
  private var blocked: Set<String> = []
  private var requests: Set<String> = []
  private var continuations: [String: CheckedContinuation<Void, Never>] = [:]
  private var requestWaiters: [String: CheckedContinuation<Void, Never>] = [:]
  private(set) var saveCalls = 0
  private var listingShouldFail = false

  init(items: [XboxSavedTouchLayout], snapshots: [String: XboxTouchLayoutSnapshot]) {
    self.items = items
    self.snapshots = snapshots
  }
  func catalog(accessHandle: String, game: CloudLibraryGame, serverRegion: String) async throws
    -> OfficialTouchCatalog
  { throw TouchLayoutError.invalid("测试未提供目录") }
  func download(
    accessHandle: String, titleID: String, bundleID: String, game: CloudLibraryGame,
    locale: String, displayScale: Double
  ) async throws -> any OfficialTouchBundleProviding { throw TouchLayoutError.invalid("测试未提供下载") }
  func savedLayouts() async throws -> [XboxSavedTouchLayout] {
    if listingShouldFail { throw TouchLayoutError.invalid("测试读取失败") }
    return items
  }
  func setListingFailure(_ value: Bool) { listingShouldFail = value }
  func save(bundle: any OfficialTouchBundleProviding, layoutID: String) async throws
    -> [XboxSavedTouchLayout]
  {
    saveCalls += 1
    return items
  }
  func load(id: String) async throws -> XboxTouchLayoutSnapshot {
    requests.insert(id)
    requestWaiters.removeValue(forKey: id)?.resume()
    if blocked.contains(id) { await withCheckedContinuation { continuations[id] = $0 } }
    guard let snapshot = snapshots[id] else { throw TouchLayoutError.invalid("测试资源损坏") }
    return snapshot
  }
  func delete(id: String) async throws -> [XboxSavedTouchLayout] {
    items.removeAll { $0.id == id }
    snapshots[id] = nil
    return items
  }
  func markUsed(id: String) async throws {}
  func blockLoad(_ id: String) {
    blocked.insert(id)
    requests.remove(id)
  }
  func releaseLoad(_ id: String) {
    blocked.remove(id)
    continuations.removeValue(forKey: id)?.resume()
  }
  func waitForLoad(_ id: String) async {
    if !requests.contains(id) { await withCheckedContinuation { requestWaiters[id] = $0 } }
  }
  func removeSnapshot(_ id: String) { snapshots[id] = nil }
  func updateRevision(_ id: String) {
    if let index = items.firstIndex(where: { $0.id == id }) {
      items[index].revision = "revision-2"
    }
  }
}
