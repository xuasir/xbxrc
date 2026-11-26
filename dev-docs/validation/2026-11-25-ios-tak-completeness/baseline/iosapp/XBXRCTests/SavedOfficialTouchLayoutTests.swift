import GameController
import UIKit
import XCTest

@testable import XBXRC

@MainActor
final class SavedOfficialTouchLayoutTests: XCTestCase {
  func testCommunityRequestsSurviveCloudAccessFailureAndCancellation() async throws {
    let game = CloudLibraryGame(gameSummary: PCPageFixtures.games[0])
    XCTAssertEqual(game.productID, "HALO00000001")
    let choice = XboxOfficialTouchBundleChoice(
      id: "community:better-xcloud:fixture", tabVersion: "fixture",
      manifestVersion: "1.0", layoutVersion: "4.1")
    let snapshot = try Self.snapshot(id: "public-layout", action: "gamepadA")
    for failure: any Error in [URLError(.timedOut), CancellationError()] {
      let calls = TouchLayoutRemoteRequestLog()
      let requests = TouchLayoutRemoteRequests(
        resolveTitle: { _, _, _ in
          await calls.record("resolve")
          throw TouchLayoutError.invalid("已知标题应直接查询")
        },
        officialCatalog: { _, _, _ in
          await calls.record("official")
          return []
        },
        communityCatalog: { titleID, productID in
          XCTAssertEqual(titleID, "1")
          XCTAssertEqual(productID, "HALO00000001")
          await calls.record("community")
          return [choice]
        },
        download: { handle, _, _, _, _, _ in
          XCTAssertNil(handle)
          await calls.record("public-download")
          return TouchLayoutTestBundle(snapshot: snapshot)
        })
      let client = RustXboxTouchLayoutClient(requests: requests)
      let access: TouchLayoutAccessProvider = {
        await calls.record("access")
        throw failure
      }
      let catalog = try await client.catalog(
        game: game, serverRegion: "auto", prepareAccess: access)
      XCTAssertEqual(catalog.bundles.map(\.id), [choice.id])
      XCTAssertNotNil(catalog.sources.first { $0.source == .official }?.failure)
      XCTAssertNil(catalog.sources.first { $0.source == .community }?.failure)
      let downloaded = try await client.download(
        titleID: catalog.titleID, bundleID: choice.id, game: game,
        locale: "zh-CN", displayScale: 2, prepareAccess: access)
      XCTAssertEqual(downloaded.choices.first?.id, snapshot.id)
      let events = await calls.values
      XCTAssertEqual(events.filter { $0 == "access" }.count, 1)
      XCTAssertEqual(events.filter { $0 == "community" }.count, 1)
      XCTAssertEqual(events.filter { $0 == "public-download" }.count, 1)
      XCTAssertFalse(events.contains("resolve"))
      XCTAssertFalse(events.contains("official"))
    }
  }

  func testResolvedTitleReusesPreparedAccessForOfficialCatalog() async throws {
    let game = CloudLibraryGame(
      gameSummary: GameSummary(
        id: "installed", titleID: "", name: "Halo Infinite", artworkURL: nil,
        lastPlayedAt: nil, playtimeMinutes: nil, achievementProgress: nil,
        productID: "HALO00000001"))
    let calls = TouchLayoutRemoteRequestLog()
    let requests = TouchLayoutRemoteRequests(
      resolveTitle: { handle, productID, _ in
        XCTAssertFalse(handle.isEmpty)
        XCTAssertEqual(productID, "HALO00000001")
        await calls.record("resolve")
        return "1"
      },
      officialCatalog: { handle, titleID, _ in
        XCTAssertFalse(handle.isEmpty)
        XCTAssertEqual(titleID, "1")
        await calls.record("official")
        return []
      },
      communityCatalog: { _, _ in
        await calls.record("community")
        return []
      },
      download: { _, _, _, _, _, _ in
        throw TouchLayoutError.invalid("当前场景只查询目录")
      })
    let client = RustXboxTouchLayoutClient(requests: requests)
    let result = try await client.catalog(
      game: game, serverRegion: "auto",
      prepareAccess: {
        await calls.record("access")
        return try await PCPageCloudClient().prepareAccess(
          session: PCPageFixtures.session, forceRegionIP: "", checkpoint: { _ in },
          authProvider: nil, startup: nil)
      })
    XCTAssertEqual(result.titleID, "1")
    XCTAssertTrue(result.sources.allSatisfy { $0.failure == nil })
    let events = await calls.values
    for event in ["access", "resolve", "official", "community"] {
      XCTAssertEqual(events.filter { $0 == event }.count, 1)
    }
  }

  func testCommunityCatalogReusesSuccessfulTitleLookupAcrossProducts() async throws {
    let calls = TouchLayoutRemoteRequestLog()
    let choice = XboxOfficialTouchBundleChoice(
      id: "community:better-xcloud:4416b1ab07588a3244a5c02ce1fa5dfa64d8df74",
      tabVersion: "4416b1ab0758", manifestVersion: "1.0", layoutVersion: "4.1")
    let snapshot = try Self.snapshot(id: "layout-1", action: "gamepadA")
    let requests = TouchLayoutRemoteRequests(
      resolveTitle: { _, _, _ in
        throw TouchLayoutError.invalid("标题身份已由目录提供")
      },
      officialCatalog: { _, _, _ in
        await calls.record("official")
        return []
      },
      communityCatalog: { titleID, _ in
        XCTAssertEqual(titleID, "2079757188")
        await calls.record("community")
        return [choice]
      },
      download: { handle, titleID, bundleID, _, _, _ in
        XCTAssertNil(handle)
        XCTAssertEqual(titleID, "2079757188")
        XCTAssertEqual(bundleID, choice.id)
        return TouchLayoutTestBundle(snapshot: snapshot)
      })
    let client = RustXboxTouchLayoutClient(requests: requests)
    let access: TouchLayoutAccessProvider = {
      await calls.record("access")
      throw URLError(.notConnectedToInternet)
    }
    for productID in ["HORIZON6BASE", "HORIZON6EDITION"] {
      let game = CloudLibraryGame(
        gameSummary: GameSummary(
          id: productID, titleID: "2079757188", name: "Forza Horizon 6", artworkURL: nil,
          lastPlayedAt: nil, playtimeMinutes: nil, achievementProgress: nil,
          productID: productID))
      let catalog = try await client.catalog(
        game: game, serverRegion: "auto", prepareAccess: access)
      XCTAssertEqual(catalog.bundles.map(\.id), [choice.id])
      XCTAssertTrue(catalog.sources.allSatisfy { $0.failure == nil })
      let bundle = try await client.download(
        titleID: catalog.titleID, bundleID: choice.id, game: game, locale: "zh-CN",
        displayScale: 2, prepareAccess: access)
      let preview = try await bundle.preview(layoutID: "layout-1")
      let prepared = try await PreparedTouchLayout.prepare(preview)
      XCTAssertTrue(prepared.layout.report.isSupported)
    }
    let events = await calls.values
    XCTAssertEqual(events.filter { $0 == "community" }.count, 1)
    XCTAssertFalse(events.contains("access"))
    XCTAssertFalse(events.contains("official"))
  }

  func testCatalogRetriesFailedSourceAndRefreshesCachedCommunityRevision() async throws {
    let game = CloudLibraryGame(gameSummary: PCPageFixtures.games[0])
    let calls = TouchLayoutRemoteRequestLog()
    let officialChoice = XboxOfficialTouchBundleChoice(
      id: "official-fixture", tabVersion: "1.0", manifestVersion: "1.0", layoutVersion: "4.1")
    let communityChoices = ["first", "refreshed"].map {
      XboxOfficialTouchBundleChoice(
        id: "community:better-xcloud:" + $0, tabVersion: $0,
        manifestVersion: "1.0", layoutVersion: "4.1")
    }
    let requests = TouchLayoutRemoteRequests(
      resolveTitle: { _, _, _ in throw TouchLayoutError.invalid("标题身份已提供") },
      officialCatalog: { _, titleID, _ in
        XCTAssertEqual(titleID, "1")
        if await calls.attempt("official") == 1 { throw URLError(.timedOut) }
        return [officialChoice]
      },
      communityCatalog: { titleID, _ in
        XCTAssertEqual(titleID, "1")
        let attempt = await calls.attempt("community")
        return [communityChoices[min(attempt - 1, 1)]]
      },
      download: { _, _, _, _, _, _ in throw TouchLayoutError.invalid("仅查询目录") })
    let client = RustXboxTouchLayoutClient(requests: requests)
    let preparedAccess = try await PCPageCloudClient().prepareAccess(
      session: PCPageFixtures.session, forceRegionIP: "", checkpoint: { _ in },
      authProvider: nil, startup: nil)
    let access: TouchLayoutAccessProvider = { preparedAccess }
    let failed = try await client.catalog(
      game: game, serverRegion: "auto", prepareAccess: access)
    XCTAssertNotNil(failed.sources.first { $0.source == .official }?.failure)
    XCTAssertEqual(failed.bundles.map(\.id), [communityChoices[0].id])
    let retried = try await client.catalog(
      game: game, serverRegion: "auto", prepareAccess: access)
    XCTAssertTrue(retried.sources.allSatisfy { $0.failure == nil })
    XCTAssertEqual(retried.bundles.map(\.id), [officialChoice.id, communityChoices[0].id])
    let refreshed = try await client.catalog(
      game: game, serverRegion: "auto", refresh: true, prepareAccess: access)
    XCTAssertEqual(refreshed.bundles.map(\.id), [officialChoice.id, communityChoices[1].id])
    let reused = try await client.catalog(
      game: game, serverRegion: "auto", prepareAccess: access)
    XCTAssertEqual(reused.bundles.map(\.id), refreshed.bundles.map(\.id))
    let events = await calls.values
    XCTAssertEqual(events.filter { $0 == "official" }.count, 4)
    XCTAssertEqual(events.filter { $0 == "community" }.count, 2)
  }

  func testCommunityCatalogRetriesFailureAndEmptyLookup() async throws {
    let game = CloudLibraryGame(gameSummary: PCPageFixtures.games[0])
    let choice = XboxOfficialTouchBundleChoice(
      id: "community:better-xcloud:recovered", tabVersion: "recovered",
      manifestVersion: "1.0", layoutVersion: "4.1")
    let calls = TouchLayoutRemoteRequestLog()
    let requests = TouchLayoutRemoteRequests(
      resolveTitle: { _, _, _ in throw TouchLayoutError.invalid("标题身份已提供") },
      officialCatalog: { _, _, _ in return [] },
      communityCatalog: { _, _ in
        switch await calls.attempt("community") {
        case 1: throw URLError(.timedOut)
        case 2: return []
        default: return [choice]
        }
      },
      download: { _, _, _, _, _, _ in throw TouchLayoutError.invalid("仅查询目录") })
    let client = RustXboxTouchLayoutClient(requests: requests)
    let preparedAccess = try await PCPageCloudClient().prepareAccess(
      session: PCPageFixtures.session, forceRegionIP: "", checkpoint: { _ in },
      authProvider: nil, startup: nil)
    let access: TouchLayoutAccessProvider = { preparedAccess }
    let failed = try await client.catalog(
      game: game, serverRegion: "auto", prepareAccess: access)
    XCTAssertNotNil(failed.sources.first { $0.source == .community }?.failure)
    let empty = try await client.catalog(
      game: game, serverRegion: "auto", prepareAccess: access)
    XCTAssertTrue(empty.bundles.isEmpty)
    XCTAssertTrue(empty.sources.allSatisfy { $0.failure == nil })
    let recovered = try await client.catalog(
      game: game, serverRegion: "auto", prepareAccess: access)
    XCTAssertEqual(recovered.bundles.map(\.id), [choice.id])
    let reused = try await client.catalog(
      game: game, serverRegion: "auto", prepareAccess: access)
    XCTAssertEqual(reused.bundles.map(\.id), [choice.id])
    let events = await calls.values
    XCTAssertEqual(events.filter { $0 == "community" }.count, 3)
  }

  func testCommunityRefreshKeepsLatestCacheWhenPreviousLookupFinishesLate() async throws {
    let game = CloudLibraryGame(gameSummary: PCPageFixtures.games[0])
    let fixture = TouchCatalogRefreshFixture()
    let requests = TouchLayoutRemoteRequests(
      resolveTitle: { _, _, _ in throw TouchLayoutError.invalid("标题身份已提供") },
      officialCatalog: { _, _, _ in return [] },
      communityCatalog: { _, _ in await fixture.catalog() },
      download: { _, _, _, _, _, _ in throw TouchLayoutError.invalid("仅查询目录") })
    let client = RustXboxTouchLayoutClient(requests: requests)
    let preparedAccess = try await PCPageCloudClient().prepareAccess(
      session: PCPageFixtures.session, forceRegionIP: "", checkpoint: { _ in },
      authProvider: nil, startup: nil)
    let access: TouchLayoutAccessProvider = { preparedAccess }
    let initial = Task {
      try await client.catalog(game: game, serverRegion: "auto", prepareAccess: access)
    }
    await fixture.waitForInitialRequest()
    let refreshed = try await client.catalog(
      game: game, serverRegion: "auto", refresh: true, prepareAccess: access)
    await fixture.completeInitialRequest()
    let previous = try await initial.value
    XCTAssertEqual(previous.bundles.first?.tabVersion, "previous")
    XCTAssertEqual(refreshed.bundles.first?.tabVersion, "latest")
    let reused = try await client.catalog(
      game: game, serverRegion: "auto", prepareAccess: access)
    XCTAssertEqual(reused.bundles.map(\.id), refreshed.bundles.map(\.id))
    let count = await fixture.requestCount
    XCTAssertEqual(count, 2)
  }

  func testOfficialAndCommunityDownloadsRetryAndPreparePreview() async throws {
    let game = CloudLibraryGame(gameSummary: PCPageFixtures.games[0])
    let snapshot = try Self.snapshot(id: "recovered", action: "gamepadA")
    for bundleID in ["official-fixture", "community:better-xcloud:fixture"] {
      let source = GameTouchLayoutSource.identify(bundleID)
      let calls = TouchLayoutRemoteRequestLog()
      let requests = TouchLayoutRemoteRequests(
        resolveTitle: { _, _, _ in throw TouchLayoutError.invalid("仅下载布局") },
        officialCatalog: { _, _, _ in return [] },
        communityCatalog: { _, _ in return [] },
        download: { handle, titleID, requestedID, _, locale, scale in
          XCTAssertEqual(handle != nil, source == .official)
          XCTAssertEqual(titleID, "1")
          XCTAssertEqual(requestedID, bundleID)
          XCTAssertEqual(locale, "zh-CN")
          XCTAssertEqual(scale, 2)
          if await calls.attempt("download") == 1 { throw URLError(.timedOut) }
          return TouchLayoutTestBundle(snapshot: snapshot)
        })
      let client = RustXboxTouchLayoutClient(requests: requests)
      let access: TouchLayoutAccessProvider = {
        await calls.record("access")
        return try await PCPageCloudClient().prepareAccess(
          session: PCPageFixtures.session, forceRegionIP: "", checkpoint: { _ in },
          authProvider: nil, startup: nil)
      }
      do {
        _ = try await client.download(
          titleID: "1", bundleID: bundleID, game: game, locale: "zh-CN",
          displayScale: 2, prepareAccess: access)
        XCTFail("首次超时应交付重试状态")
      } catch {
        XCTAssertEqual((error as? URLError)?.code, .timedOut)
      }
      let recovered = try await client.download(
        titleID: "1", bundleID: bundleID, game: game, locale: "zh-CN",
        displayScale: 2, prepareAccess: access)
      let preview = try await recovered.preview(layoutID: snapshot.id)
      let prepared = try await PreparedTouchLayout.prepare(preview)
      XCTAssertTrue(prepared.layout.report.isSupported)
      let events = await calls.values
      XCTAssertEqual(events.filter { $0 == "download" }.count, 2)
      XCTAssertEqual(events.filter { $0 == "access" }.count, source == .official ? 2 : 0)
    }
  }

  func testCloudQueriesOnlyAndReusesSelectedPackageWithoutChangingHomeRecent() async throws {
    let (controller, session, library, _) = try await setup()
    await assertSelection(.saved("first"), controller: controller, library: library, expected: true)
    let homeRecent = controller.recentCustomSelection
    var persisted: [String?] = []
    controller.onRecentSelectionChanged = { persisted.append($0) }
    let calls = TouchLayoutRemoteRequestLog()
    let (client, access) = try await cloudTestClient(calls: calls)
    controller.beginSession(generation: 2, target: .cloud)
    controller.setCloudGame(CloudLibraryGame(gameSummary: PCPageFixtures.games[0]), generation: 2)
    await controller.loadCloudCatalog(
      client: client, serverRegion: "auto", locale: "zh-CN", displayScale: 2,
      prepareAccess: access)
    XCTAssertEqual(controller.cloudCatalog?.bundles.count, 2)
    XCTAssertEqual(controller.cloudDownloadedBundleCount, 0)
    let queried = await calls.values
    XCTAssertFalse(queried.contains { $0.hasPrefix("download:") })
    await assertSelection(
      .saved("second"), controller: controller, library: library, expected: false)
    XCTAssertEqual(controller.selection, .standard)
    await assertSelection(
      .gameBundle("official-cloud"), controller: controller, library: library, expected: true)
    XCTAssertEqual(
      controller.selection, .gameLayout(bundleID: "official-cloud", layoutID: "default"))
    XCTAssertEqual(controller.cloudLayoutChoices.count, 2)
    _ = session.outputState(now: 1)
    let a = try XCTUnwrap(
      controller.input.sockets.first { $0.control.action == .gamepad("gamepadA") })
    controller.input.process([.init(id: 1, phase: .began, position: a.center)])
    XCTAssertEqual(session.outputState(now: 2).buttonMask, 16)
    let alternative = TouchLayoutSelection.gameLayout(
      bundleID: "official-cloud", layoutID: "alternative")
    await assertSelection(alternative, controller: controller, library: library, expected: true)
    XCTAssertEqual(session.outputState(now: 3), .neutral)
    XCTAssertEqual(controller.input.gamepad, .neutral)
    let b = try XCTUnwrap(
      controller.input.sockets.first { $0.control.action == .gamepad("gamepadB") })
    controller.input.process([.init(id: 2, phase: .began, position: b.center)])
    XCTAssertEqual(session.outputState(now: 4).buttonMask, 32)
    XCTAssertEqual(controller.nextQuickSelection(preferences: .init(), library: library), .standard)
    await assertSelection(.standard, controller: controller, library: library, expected: true)
    XCTAssertEqual(
      controller.nextQuickSelection(preferences: .init(), library: library), alternative)
    await assertSelection(alternative, controller: controller, library: library, expected: true)
    let downloaded = await calls.values
    XCTAssertEqual(downloaded.filter { $0 == "download:official-cloud" }.count, 1)
    XCTAssertEqual(controller.recentCustomSelection, homeRecent)
    XCTAssertTrue(persisted.isEmpty)
    controller.beginSession(generation: 3, target: .home)
    XCTAssertNil(controller.cloudGame)
    XCTAssertEqual(controller.cloudDownloadedBundleCount, 0)
    XCTAssertEqual(
      controller.nextQuickSelection(preferences: .init(), library: library), .saved("first"))
  }

  func testCloudDownloadFailureKeepsCurrentLayoutAndRetrySwitchesSource() async throws {
    let (controller, session, library, _) = try await setup()
    let calls = TouchLayoutRemoteRequestLog()
    let (client, access) = try await cloudTestClient(calls: calls, failCommunityOnce: true)
    controller.beginSession(generation: 2, target: .cloud)
    controller.setCloudGame(CloudLibraryGame(gameSummary: PCPageFixtures.games[0]), generation: 2)
    await controller.loadCloudCatalog(
      client: client, serverRegion: "auto", locale: "zh-CN", displayScale: 2,
      prepareAccess: access)
    await assertSelection(
      .gameBundle("official-cloud"), controller: controller, library: library, expected: true)
    let previous = controller.selection
    let communityID = "community:better-xcloud:cloud"
    await assertSelection(
      .gameBundle(communityID), controller: controller, library: library, expected: false)
    XCTAssertEqual(controller.selection, previous)
    XCTAssertNotNil(controller.cloudDownloadFailures[communityID])
    await assertSelection(
      .gameBundle(communityID), controller: controller, library: library, expected: true)
    XCTAssertEqual(controller.selection, .gameLayout(bundleID: communityID, layoutID: "default"))
    XCTAssertNil(controller.cloudDownloadFailures[communityID])
    _ = session.outputState(now: 1)
    let b = try XCTUnwrap(
      controller.input.sockets.first { $0.control.action == .gamepad("gamepadB") })
    controller.input.process([.init(id: 3, phase: .began, position: b.center)])
    XCTAssertEqual(session.outputState(now: 2).buttonMask, 32)
    let events = await calls.values
    XCTAssertEqual(events.filter { $0 == "download:" + communityID }.count, 2)
  }

  func testLateCloudDownloadCannotOverwriteHomeSession() async throws {
    let (controller, _, library, _) = try await setup()
    let gate = TouchCloudDownloadGate()
    let (client, access) = try await cloudTestClient(
      calls: TouchLayoutRemoteRequestLog(), gate: gate)
    controller.beginSession(generation: 2, target: .cloud)
    controller.setCloudGame(CloudLibraryGame(gameSummary: PCPageFixtures.games[0]), generation: 2)
    await controller.loadCloudCatalog(
      client: client, serverRegion: "auto", locale: "zh-CN", displayScale: 2,
      prepareAccess: access)
    let pending = Task {
      await controller.select(.gameBundle("official-cloud"), library: library, preferences: .init())
    }
    await gate.waitForDownload()
    controller.beginSession(generation: 3, target: .home)
    await assertSelection(
      .saved("second"), controller: controller, library: library, expected: true)
    await gate.completeDownload()
    let completed = await pending.value
    XCTAssertFalse(completed)
    XCTAssertEqual(controller.selection, .saved("second"))
    XCTAssertEqual(controller.cloudDownloadedBundleCount, 0)
    XCTAssertNil(controller.cloudCatalog)
  }

  func testCloudUsesSupportedAlternativeWhenDefaultNeedsAdaptation() async throws {
    let (controller, _, library, _) = try await setup()
    let (client, access) = try await cloudTestClient(
      calls: TouchLayoutRemoteRequestLog(), defaultIssues: ["默认布局等待适配"])
    controller.beginSession(generation: 2, target: .cloud)
    controller.setCloudGame(CloudLibraryGame(gameSummary: PCPageFixtures.games[0]), generation: 2)
    await controller.loadCloudCatalog(
      client: client, serverRegion: "auto", locale: "zh-CN", displayScale: 2,
      prepareAccess: access)
    await assertSelection(
      .gameBundle("official-cloud"), controller: controller, library: library, expected: true)
    XCTAssertEqual(
      controller.selection, .gameLayout(bundleID: "official-cloud", layoutID: "alternative"))
    XCTAssertEqual(controller.cloudDownloadedBundleCount, 1)
  }

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

  func testVPrefixedTouchpadPassesSaveCapabilityGate() async throws {
    var snapshot = try Self.snapshot(id: "second", action: "gamepadB")
    snapshot.documentJson =
      #"{"$schema":"https://schema.play.xbox.com/layout/v3.1/layout.json","content":{"right":{"inner":[{"type":"touchpad","action":"gamepadB","renderAsButton":true,"axis":{"input":"axisXY","output":"rightJoystick"}}]}}}"#
    let prepared = try await PreparedTouchLayout.prepare(snapshot)
    XCTAssertTrue(prepared.layout.report.isSupported, "\(prepared.layout.report.issues)")
    let client = TouchLayoutTestClient(items: [], snapshots: [:])
    let library = SavedTouchLayoutsStore(client: client)
    try await library.save(bundle: TouchLayoutTestBundle(snapshot: snapshot), prepared: prepared)
    let writes = await client.saveCalls
    XCTAssertEqual(writes, 1)
  }

  private func prepareArtwork(_ content: [String: Any]) async throws -> PreparedTouchLayout {
    var snapshot = try Self.snapshot(id: "artwork", action: "gamepadA")
    snapshot.documentJson = String(
      decoding: try JSONSerialization.data(withJSONObject: [
        "$schema": "https://schema.play.xbox.com/layout/v3.1/layout.json", "content": content,
      ]), as: UTF8.self)
    snapshot.assets = [("red.png", UIColor.red), ("blue.png", UIColor.blue)].map { id, color in
      .init(
        id: id,
        data: UIGraphicsImageRenderer(size: CGSize(width: 8, height: 8)).pngData {
          color.setFill()
          $0.fill(CGRect(x: 0, y: 0, width: 8, height: 8))
        })
    }
    return try await PreparedTouchLayout.prepare(snapshot)
  }

  func testOfficialBackgroundAndMovingKnobReuseDecodedImages() async throws {
    let redAsset: [String: Any] = ["type": "asset", "value": "red.png"]
    let blueAsset: [String: Any] = ["type": "asset", "value": "blue.png"]
    let prepared = try await prepareArtwork([
      "left": [
        "inner": [
          [
            "type": "joystick", "action": "leftThumb",
            "axis": ["input": "axisXY", "output": "leftJoystick"],
            "styles": [
              "default": [
                "opacity": 0.5, "background": redAsset,
                "knob": ["background": redAsset, "faceImage": blueAsset, "opacity": 0.6],
              ],
              "moving": ["background": blueAsset, "knob": ["background": blueAsset]],
              "activated": ["background": redAsset],
            ],
          ],
          ["type": "directionalPad", "styles": ["default": ["background": blueAsset]]],
        ]
      ],
      "right": [
        "inner": [
          [
            "type": "touchpad", "action": "gamepadB", "renderAsButton": true,
            "axis": ["input": "axisXY", "output": "rightJoystick"],
            "styles": [
              "default": ["background": redAsset, "faceImage": blueAsset],
              "activated": ["background": blueAsset],
            ],
          ],
          [
            "type": "throttle", "axisUp": "rightTrigger", "axisDown": "leftTrigger",
            "styles": [
              "default": [
                "knob": ["background": redAsset, "faceImage": blueAsset],
                "axisUp": ["cap": ["type": "color", "value": "#0f08", "opacity": 0.4]],
                "axisDown": ["cap": ["type": "color", "value": "#ff000080"]],
              ]
            ],
          ],
        ]
      ],
    ])
    XCTAssertTrue(prepared.layout.report.isSupported, "\(prepared.layout.report.issues)")
    let red = try XCTUnwrap(prepared.assets.images["red.png"])
    let blue = try XCTUnwrap(prepared.assets.images["blue.png"])
    let input = TouchInputAccumulator()
    input.load(prepared.layout)
    let surface = TouchLayoutSurfaceView(input: input)
    defer { surface.stop() }
    surface.assets = prepared.assets
    surface.frame = CGRect(x: 0, y: 0, width: 800, height: 420)
    surface.layoutSubviews()
    let stick = try XCTUnwrap(input.sockets.first { $0.control.kind == .joystick })
    let drawing = try XCTUnwrap(surface.layer.sublayers?.first { $0.position == stick.center })
    let background = try XCTUnwrap(drawing.sublayers?.first { $0.name == "control-background" })
    let knob = try XCTUnwrap(drawing.sublayers?.first { $0.name == "knob-background" })
    XCTAssertTrue(try XCTUnwrap(background.contents) as AnyObject === red)
    XCTAssertTrue(try XCTUnwrap(knob.contents) as AnyObject === red)
    XCTAssertEqual(knob.opacity, 0.6, accuracy: 0.001)
    XCTAssertEqual(drawing.opacity, surface.controlOpacity * 0.5, accuracy: 0.001)
    input.process([.init(id: 1, phase: .began, position: stick.center)])
    XCTAssertTrue(try XCTUnwrap(background.contents) as AnyObject === blue)
    XCTAssertTrue(try XCTUnwrap(knob.contents) as AnyObject === blue)
    input.process([
      .init(
        id: 1, phase: .moved,
        position: CGPoint(
          x: stick.center.x + stick.diameter, y: stick.center.y))
    ])
    XCTAssertTrue(try XCTUnwrap(background.contents) as AnyObject === red)
    XCTAssertEqual(input.gamepad.buttonMask & 16384, 16384)
    let pad = try XCTUnwrap(input.sockets.first { $0.control.kind == .touchpad })
    input.process([.init(id: 2, phase: .began, position: pad.center)])
    let padDrawing = try XCTUnwrap(surface.layer.sublayers?.first { $0.position == pad.center })
    XCTAssertTrue(
      try XCTUnwrap(padDrawing.sublayers?.first { $0.name == "control-background" }?.contents)
        as AnyObject === blue)
    let throttle = try XCTUnwrap(input.sockets.first { $0.control.kind == .throttle })
    let throttleDrawing = try XCTUnwrap(
      surface.layer.sublayers?.first { $0.position == throttle.center })
    let cap = try XCTUnwrap(throttleDrawing.sublayers?.first { $0.name == "axis-up-cap" })
    XCTAssertFalse(cap.isHidden)
    XCTAssertEqual(cap.opacity, 0.4, accuracy: 0.001)
    XCTAssertEqual(try XCTUnwrap(cap.backgroundColor?.alpha), 136.0 / 255, accuracy: 0.001)
    let attachment = XCTAttachment(
      image: UIGraphicsImageRenderer(size: surface.bounds.size).image {
        surface.layer.render(in: $0.cgContext)
      })
    attachment.name = "official-background-and-knob-assets"
    attachment.lifetime = .keepAlways
    add(attachment)
    input.cancelAll()
    XCTAssertEqual(input.gamepad, .neutral)
    XCTAssertTrue(try XCTUnwrap(background.contents) as AnyObject === red)
    XCTAssertTrue(try XCTUnwrap(knob.contents) as AnyObject === red)
    input.load(try TouchLayoutCatalog.read("generic")[0])
    XCTAssertNil(background.contents)
    XCTAssertTrue(background.isHidden)
    XCTAssertNil(knob.contents)
    XCTAssertTrue(knob.isHidden)
  }

  func testPullIndicatorUsesActiveSegmentAndReleasesItsArtwork() async throws {
    let prepared = try await prepareArtwork([
      "right": [
        "inner": [
          [
            "type": "button", "action": "gamepadA",
            "pullAction": [
              "type": "segmented", "items": ["gamepadX", "gamepadB", "gamepadY", "leftTrigger"],
            ],
            "styles": [
              "default": [
                "background": ["type": "asset", "value": "red.png"],
                "pullIndicator": [
                  [
                    "opacity": 0.5, "background": ["type": "color", "value": "#f008"],
                    "faceImage": ["type": "asset", "value": "red.png", "opacity": 0.4],
                  ],
                  NSNull(),
                  [
                    "background": ["type": "color", "value": "#0000ff80"],
                    "faceImage": ["type": "asset", "value": "blue.png"],
                  ],
                  NSNull(),
                ],
              ],
              "pulled": ["background": ["type": "asset", "value": "blue.png"]],
            ],
          ]
        ]
      ]
    ])
    XCTAssertTrue(prepared.layout.report.isSupported, "\(prepared.layout.report.issues)")
    let red = try XCTUnwrap(prepared.assets.images["red.png"])
    let blue = try XCTUnwrap(prepared.assets.images["blue.png"])
    let input = TouchInputAccumulator()
    input.load(prepared.layout)
    let surface = TouchLayoutSurfaceView(input: input)
    defer { surface.stop() }
    surface.assets = prepared.assets
    surface.frame = CGRect(x: 0, y: 0, width: 800, height: 420)
    surface.layoutSubviews()
    let button = try XCTUnwrap(input.sockets.first)
    let drawing = try XCTUnwrap(surface.layer.sublayers?.first)
    let up = CGPoint(x: button.center.x, y: button.center.y - button.unitSize)
    input.process([
      .init(id: 1, phase: .began, position: button.center),
      .init(id: 1, phase: .moved, position: up),
    ])
    XCTAssertEqual(input.gamepad.buttonMask, 16 | 64)
    let pull = try XCTUnwrap(drawing.sublayers?.first { $0.name == "pull-indicator-1" })
    XCTAssertLessThan(pull.position.y, drawing.bounds.midY)
    XCTAssertEqual(pull.opacity, 0.5, accuracy: 0.001)
    XCTAssertEqual(try XCTUnwrap(pull.sublayers?[3]).opacity, 0.4, accuracy: 0.001)
    XCTAssertTrue(try XCTUnwrap(pull.sublayers?[3].contents) as AnyObject === red)
    let down = CGPoint(x: button.center.x, y: button.center.y + button.unitSize)
    input.process([.init(id: 1, phase: .moved, position: down)])
    XCTAssertEqual(input.gamepad.buttonMask, 16 | 128)
    XCTAssertGreaterThan(pull.position.y, drawing.bounds.midY)
    XCTAssertTrue(try XCTUnwrap(pull.sublayers?[3].contents) as AnyObject === blue)
    input.cancelAll()
    XCTAssertEqual(input.gamepad, .neutral)
    XCTAssertFalse(drawing.sublayers?.contains { $0.name == "pull-indicator-1" } ?? true)
  }

  func testUnknownImageLocationsKeepSaveGateAndDeduplicateMessages() async throws {
    let asset: [String: Any] = ["type": "asset", "value": "red.png"]
    let prepared = try await prepareArtwork([
      "right": [
        "inner": [
          ["type": "button", "action": "gamepadA", "styles": ["future": ["background": asset]]],
          [
            "type": "button", "action": "gamepadB",
            "styles": ["default": ["knob": ["background": asset]]],
          ],
          [
            "type": "button", "action": "gamepadX", "pullAction": "gamepadY",
            "styles": ["default": ["pullIndicator": ["background": asset]]],
          ],
          [
            "type": "throttle", "axisUp": "rightTrigger",
            "styles": ["default": ["axisUp": ["cap": asset]]],
          ],
        ]
      ]
    ])
    XCTAssertFalse(prepared.layout.report.isSupported)
    XCTAssertEqual(prepared.layout.report.issues.filter { $0.severity == .unsupported }.count, 4)
    XCTAssertEqual(prepared.layout.report.unsupportedMessages, ["此图片位置等待绘制支持"])
    let client = TouchLayoutTestClient(items: [], snapshots: [:])
    let library = SavedTouchLayoutsStore(client: client)
    let snapshot = try Self.snapshot(id: prepared.id, action: "gamepadA")
    do {
      try await library.save(bundle: TouchLayoutTestBundle(snapshot: snapshot), prepared: prepared)
      XCTFail("未知图片位置需要阻止保存")
    } catch {}
    let writes = await client.saveCalls
    XCTAssertEqual(writes, 0)
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

  private func assertSelection(
    _ choice: TouchLayoutSelection, controller: StreamingTouchLayoutController,
    library: SavedTouchLayoutsStore, expected: Bool,
    file: StaticString = #filePath, line: UInt = #line
  ) async {
    let applied = await controller.select(choice, library: library, preferences: .init())
    XCTAssertEqual(applied, expected, file: file, line: line)
  }

  private func cloudTestClient(
    calls: TouchLayoutRemoteRequestLog, failCommunityOnce: Bool = false,
    gate: TouchCloudDownloadGate? = nil, defaultIssues: [String] = []
  ) async throws -> (RustXboxTouchLayoutClient, TouchLayoutAccessProvider) {
    var defaultSnapshot = try Self.snapshot(id: "default", action: "gamepadA")
    defaultSnapshot.issues = defaultIssues
    let official = TouchLayoutMultiChoiceTestBundle(snapshots: [
      defaultSnapshot,
      try Self.snapshot(id: "alternative", action: "gamepadB"),
    ])
    let community = TouchLayoutMultiChoiceTestBundle(snapshots: [
      try Self.snapshot(id: "default", action: "gamepadB"),
      try Self.snapshot(id: "alternative", action: "gamepadA"),
    ])
    let requests = TouchLayoutRemoteRequests(
      resolveTitle: { _, _, _ in throw TouchLayoutError.invalid("当前游戏已提供标题身份") },
      officialCatalog: { _, _, _ in
        await calls.record("official-catalog")
        return [
          .init(
            id: "official-cloud", tabVersion: "1.0", manifestVersion: "1.0", layoutVersion: "4.1")
        ]
      },
      communityCatalog: { _, _ in
        await calls.record("community-catalog")
        return [
          .init(
            id: "community:better-xcloud:cloud", tabVersion: "cloud", manifestVersion: "1.0",
            layoutVersion: "4.1")
        ]
      },
      download: { _, titleID, bundleID, game, _, _ in
        XCTAssertEqual(titleID, "1")
        XCTAssertEqual(game.productID, "HALO00000001")
        let attempt = await calls.attempt("download:" + bundleID)
        if let gate { await gate.startDownload() }
        if GameTouchLayoutSource.identify(bundleID) == .community {
          if failCommunityOnce && attempt == 1 { throw URLError(.timedOut) }
          return community
        }
        return official
      })
    let prepared = try await PCPageCloudClient().prepareAccess(
      session: PCPageFixtures.session, forceRegionIP: "", checkpoint: { _ in },
      authProvider: nil, startup: nil)
    return (RustXboxTouchLayoutClient(requests: requests), { prepared })
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

private actor TouchLayoutRemoteRequestLog {
  private(set) var values: [String] = []
  func record(_ value: String) { values.append(value) }
  func attempt(_ value: String) -> Int {
    values.append(value)
    return values.filter { $0 == value }.count
  }
}

private actor TouchCatalogRefreshFixture {
  private(set) var requestCount = 0
  private var initial: CheckedContinuation<[XboxOfficialTouchBundleChoice], Never>?
  private var waiter: CheckedContinuation<Void, Never>?

  func catalog() async -> [XboxOfficialTouchBundleChoice] {
    requestCount += 1
    if requestCount > 1 { return [choice("latest")] }
    return await withCheckedContinuation {
      initial = $0
      waiter?.resume()
      waiter = nil
    }
  }
  func waitForInitialRequest() async {
    if initial == nil { await withCheckedContinuation { waiter = $0 } }
  }
  func completeInitialRequest() {
    initial?.resume(returning: [choice("previous")])
    initial = nil
  }
  private func choice(_ revision: String) -> XboxOfficialTouchBundleChoice {
    .init(
      id: "community:better-xcloud:" + revision, tabVersion: revision,
      manifestVersion: "1.0", layoutVersion: "4.1")
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

private struct TouchLayoutMultiChoiceTestBundle: OfficialTouchBundleProviding {
  let snapshots: [XboxTouchLayoutSnapshot]
  var version: String { "1.0" }
  var choices: [XboxTouchLayoutChoice] {
    snapshots.enumerated().map { index, snapshot in
      .init(id: snapshot.id, name: snapshot.name, isDefault: index == 0, issues: snapshot.issues)
    }
  }
  func preview(layoutID: String) async throws -> XboxTouchLayoutSnapshot {
    guard let snapshot = snapshots.first(where: { $0.id == layoutID }) else {
      throw TouchLayoutError.invalid("布局身份暂缺")
    }
    return snapshot
  }
}

private actor TouchCloudDownloadGate {
  private var download: CheckedContinuation<Void, Never>?
  private var waiter: CheckedContinuation<Void, Never>?
  func startDownload() async {
    await withCheckedContinuation {
      download = $0
      waiter?.resume()
      waiter = nil
    }
  }
  func waitForDownload() async {
    if download == nil { await withCheckedContinuation { waiter = $0 } }
  }
  func completeDownload() {
    download?.resume()
    download = nil
  }
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
  func catalog(
    game: CloudLibraryGame, serverRegion: String, refresh: Bool,
    prepareAccess: @escaping TouchLayoutAccessProvider
  ) async throws
    -> OfficialTouchCatalog
  { throw TouchLayoutError.invalid("测试未提供目录") }
  func download(
    titleID: String, bundleID: String, game: CloudLibraryGame,
    locale: String, displayScale: Double, prepareAccess: @escaping TouchLayoutAccessProvider
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
