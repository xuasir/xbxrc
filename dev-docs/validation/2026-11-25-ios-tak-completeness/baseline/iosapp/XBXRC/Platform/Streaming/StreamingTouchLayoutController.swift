import Combine
import Foundation

enum TouchLayoutSelection: Equatable, Sendable {
  case preferences
  case standard
  case template(String)
  case saved(String)
  case gameBundle(String)
  case gameLayout(bundleID: String, layoutID: String)

  var persistentID: String? {
    switch self {
    case .template(let id): "template:" + id
    case .saved(let id): id
    case .standard, .preferences, .gameBundle, .gameLayout: nil
    }
  }

  static func fromPersistentID(_ id: String?) -> Self? {
    guard let id, id.count <= 200 else { return nil }
    if id.hasPrefix("template:"), let uuid = UUID(uuidString: String(id.dropFirst(9))) {
      return .template(uuid.uuidString)
    }
    if id.hasPrefix("official:") || id.hasPrefix("community:") { return .saved(id) }
    return nil
  }

  var source: String {
    switch self {
    case .preferences: "preferences"
    case .standard: "standard"
    case .template: "template"
    case .saved(let id): id.hasPrefix("community:") ? "community" : "official"
    case .gameBundle(let id), .gameLayout(let id, _):
      GameTouchLayoutSource.identify(id).rawValue
    }
  }

  var isGameSelection: Bool {
    switch self {
    case .gameBundle, .gameLayout: true
    default: false
    }
  }
}

@MainActor
final class StreamingTouchLayoutController: ObservableObject {
  let input = TouchInputAccumulator()
  @Published private(set) var selection: TouchLayoutSelection = .preferences
  @Published private(set) var pendingSelection: TouchLayoutSelection?
  @Published private(set) var recentCustomSelection: TouchLayoutSelection?
  var onRecentSelectionChanged: ((String?) -> Void)?
  @Published private(set) var assets = TouchLayoutAssets.empty
  @Published private(set) var loadError: String?
  @Published private(set) var streamTarget: StreamingLaunchTarget = .home
  @Published private(set) var cloudGame: CloudLibraryGame?
  @Published private(set) var cloudCatalog: OfficialTouchCatalog?
  @Published private(set) var cloudCatalogLoading = false
  @Published private(set) var cloudCatalogFailure: SectionRequestFailure?
  @Published private(set) var cloudDownloadFailures: [String: SectionRequestFailure] = [:]
  @Published private var cloudDownloadedBundles: [String: any OfficialTouchBundleProviding] = [:]
  private let standard: TouchLayout?
  private var sessionGeneration: UInt64?
  private var selectionRevision: UInt64 = 0
  private var savedRevision: String?
  private weak var session: StreamInputSession?
  private var suppressionSubscription: AnyCancellable?
  private var cloudCatalogRevision: UInt64 = 0
  private var cloudCatalogContext: String?
  private var recentGameSelection: TouchLayoutSelection?
  private var cloudDownload: (@Sendable (String) async throws -> any OfficialTouchBundleProviding)?

  var cloudDownloadedBundleCount: Int { cloudDownloadedBundles.count }
  var cloudLayoutChoices: [StreamingGameTouchLayoutChoice] {
    (cloudCatalog?.bundles ?? []).flatMap { descriptor in
      guard let bundle = cloudDownloadedBundles[descriptor.id] else {
        return [StreamingGameTouchLayoutChoice]()
      }
      return bundle.choices.map {
        StreamingGameTouchLayoutChoice(
          bundleID: descriptor.id, version: bundle.version, choice: $0)
      }
    }
  }

  init(bundle: Bundle = .main) {
    do {
      let layout = try TouchLayoutCatalog.standard(bundle: bundle)
      standard = layout
      input.load(layout)
    } catch {
      standard = nil
      loadError = error.localizedDescription
    }
  }

  func bind(to session: StreamInputSession) {
    guard self.session !== session else { return }
    if let previous = self.session { unbind(from: previous) }
    self.session = session
    input.onInput = { [weak session] frame in
      session?.setTouchLayoutState(frame.gamepad, active: frame.pointerCount > 0)
    }
    suppressionSubscription = session.$isSuppressed.removeDuplicates().sink {
      [weak self] value in
      if value { self?.input.cancelAll(reason: "inputSuppressed") }
    }
  }

  func unbind(from session: StreamInputSession) {
    guard self.session === session else { return }
    input.cancelAll(reason: "streamLayoutClosed")
    input.onInput = nil
    suppressionSubscription = nil
    session.releaseTouches()
    self.session = nil
  }

  func beginSession(
    generation: UInt64, preferences: MobilePreferences = .init(),
    target: StreamingLaunchTarget = .home
  ) {
    sessionGeneration = generation
    cancelPendingSelection()
    streamTarget = target
    cloudGame = nil
    resetCloudResources()
    savedRevision = nil
    if let standard {
      commit(standard, assets: .empty, selection: target == .cloud ? .standard : .preferences)
    }
    if target == .home { rememberCustomSelection(effectiveSelection(preferences: preferences)) }
  }

  func setCloudGame(_ game: CloudLibraryGame, generation: UInt64) {
    guard streamTarget == .cloud, sessionGeneration == generation else { return }
    guard cloudGame?.productID != game.productID || cloudGame?.xboxTitleID != game.xboxTitleID
    else {
      return
    }
    let hadGame = cloudGame != nil
    cancelPendingSelection()
    cloudGame = game
    resetCloudResources()
    if hadGame, let standard { commit(standard, assets: .empty, selection: .standard) }
  }

  /// Cloud 先交付目录，下载闭包留给点选操作；成功包仅在当前会话复用。
  func loadCloudCatalog(
    client: any XboxTouchLayoutClient, serverRegion: String, locale: String,
    displayScale: Double, refresh: Bool = false,
    prepareAccess: @escaping TouchLayoutAccessProvider
  ) async {
    guard streamTarget == .cloud, let game = cloudGame, let generation = sessionGeneration else {
      return
    }
    let context = "\(game.productID):\(serverRegion):\(locale):\(displayScale)"
    if !refresh, cloudCatalog != nil, cloudCatalogContext == context { return }
    if pendingSelection?.isGameSelection == true { cancelPendingSelection() }
    cloudCatalogRevision &+= 1
    let request = cloudCatalogRevision
    cloudCatalogLoading = true
    cloudCatalogFailure = nil
    defer { if request == cloudCatalogRevision { cloudCatalogLoading = false } }
    do {
      let catalog = try await client.catalog(
        game: game, serverRegion: serverRegion, refresh: refresh, prepareAccess: prepareAccess)
      try Task.checkCancellation()
      guard request == cloudCatalogRevision, generation == sessionGeneration,
        cloudGame?.productID == game.productID
      else { return }
      let previous = cloudCatalog?.bundles ?? []
      cloudDownloadedBundles = cloudDownloadedBundles.filter { id, _ in
        context == cloudCatalogContext
          && catalog.bundles.contains { current in
            current.id == id
              && previous.contains {
                $0.id == id && $0.tabVersion == current.tabVersion
                  && $0.manifestVersion == current.manifestVersion
                  && $0.layoutVersion == current.layoutVersion
              }
          }
      }
      cloudCatalog = catalog
      cloudCatalogContext = context
      cloudDownloadFailures = [:]
      cloudDownload = { bundleID in
        try await client.download(
          titleID: catalog.titleID, bundleID: bundleID, game: game, locale: locale,
          displayScale: displayScale, prepareAccess: prepareAccess)
      }
    } catch {
      guard request == cloudCatalogRevision, generation == sessionGeneration,
        !Task.isCancelled
      else { return }
      cloudCatalogFailure = SectionRequestFailure(error: error)
    }
  }

  private func resetCloudResources() {
    cloudCatalogRevision &+= 1
    cloudCatalog = nil
    cloudCatalogLoading = false
    cloudCatalogFailure = nil
    cloudDownloadFailures = [:]
    cloudDownloadedBundles = [:]
    cloudCatalogContext = nil
    cloudDownload = nil
    recentGameSelection = nil
  }

  func endSession() {
    sessionGeneration = nil
    cancelPendingSelection()
    cloudGame = nil
    resetCloudResources()
    input.cancelAll(reason: "touchLayoutSessionEnded")
    session?.releaseTouches()
    selection = .preferences
    savedRevision = nil
    assets = .empty
  }

  func surfaceClosed() {
    input.cancelAll(reason: "streamLayoutSurfaceClosed")
    session?.releaseTouches()
  }

  func effectiveSelection(preferences: MobilePreferences) -> TouchLayoutSelection {
    if selection == .preferences {
      return preferences.selectedTouchTemplate.map { .template($0.id) } ?? .standard
    }
    return selection
  }

  func restoreRecentSelection(preferences: MobilePreferences) {
    recentCustomSelection = TouchLayoutSelection.fromPersistentID(
      preferences.recentCustomTouchLayoutID)
  }

  func mostRecentCustomSelection(
    preferences: MobilePreferences, library: SavedTouchLayoutsStore
  ) -> TouchLayoutSelection? {
    if streamTarget == .cloud {
      return recentGameSelection.flatMap {
        isAvailable($0, preferences: preferences, library: library) ? $0 : nil
      }
    }
    if let recentCustomSelection,
      isAvailable(recentCustomSelection, preferences: preferences, library: library)
    {
      return recentCustomSelection
    }
    if let saved = library.layouts.filter({ $0.lastUsedAtMs > 0 && library.isAvailable($0.id) })
      .max(by: { $0.lastUsedAtMs < $1.lastUsedAtMs })
    {
      return .saved(saved.id)
    }
    return preferences.selectedTouchTemplate.map { .template($0.id) }
  }

  func nextQuickSelection(
    preferences: MobilePreferences, library: SavedTouchLayoutsStore
  ) -> TouchLayoutSelection? {
    // 准备阶段再次点击时，按待应用项循环，立即取消旧加载并切回标准。
    let current = pendingSelection ?? effectiveSelection(preferences: preferences)
    return current == .standard
      ? mostRecentCustomSelection(preferences: preferences, library: library) : .standard
  }

  private func isAvailable(
    _ choice: TouchLayoutSelection, preferences: MobilePreferences,
    library: SavedTouchLayoutsStore
  ) -> Bool {
    switch choice {
    case .template(let id): preferences.touchLayoutTemplates.contains { $0.id == id }
    case .saved(let id): library.isAvailable(id)
    case .gameLayout(let bundleID, let layoutID):
      streamTarget == .cloud
        && cloudLayoutChoices.contains {
          $0.bundleID == bundleID && $0.choice.id == layoutID
        }
    case .gameBundle: false
    case .standard, .preferences: false
    }
  }

  func configuration(
    preferences: MobilePreferences, orientation: TouchLayoutOrientation
  ) -> TouchLayoutConfiguration {
    if selection == .preferences { return preferences.touchConfiguration(for: orientation) }
    if case .template(let id) = selection,
      let template = preferences.touchLayoutTemplates.first(where: { $0.id == id })
    {
      return .init(
        identity: id, positions: template.positions(for: orientation),
        scale: template.appearance.scale)
    }
    return .init(
      identity: savedRevision ?? TouchLayoutTemplate.standardID, scale: preferences.touchScale
    )
  }

  func opacity(preferences: MobilePreferences) -> Double {
    if selection == .preferences { return preferences.selectedTouchAppearance.opacity }
    if case .template(let id) = selection,
      let template = preferences.touchLayoutTemplates.first(where: { $0.id == id })
    {
      return template.appearance.opacity
    }
    return preferences.touchOpacity
  }

  @discardableResult
  func select(
    _ next: TouchLayoutSelection, library: SavedTouchLayoutsStore,
    preferences: MobilePreferences
  ) async -> Bool {
    guard
      streamTarget == .cloud
        ? (next == .standard || next.isGameSelection) : !next.isGameSelection
    else {
      loadError = "请选择当前会话的布局"
      return false
    }
    selectionRevision &+= 1
    let request = selectionRevision
    let generation = sessionGeneration
    let catalogRevision = cloudCatalogRevision
    guard generation != nil else { return false }
    pendingSelection = next
    loadError = nil
    let startedAt = Date()
    trace("touchLayoutPrepareStarted", source: next.source, revision: request)
    do {
      let prepared: PreparedTouchLayout
      var item: XboxSavedTouchLayout?
      var appliedSelection = next
      var resourceRevision: String?
      switch next {
      case .saved(let id):
        guard let saved = library.layouts.first(where: { $0.id == id }) else {
          throw TouchLayoutError.invalid("已保存布局暂缺")
        }
        item = saved
        prepared = try await library.load(saved)
      case .template(let id):
        guard preferences.touchLayoutTemplates.contains(where: { $0.id == id }) else {
          throw TouchLayoutError.invalid("自定义模板已删除")
        }
        prepared = try standardSnapshot()
      case .standard, .preferences:
        prepared = try standardSnapshot()
      case .gameBundle(let bundleID), .gameLayout(let bundleID, _):
        let bundle = try await loadCloudBundle(
          bundleID, request: request, generation: generation, catalogRevision: catalogRevision)
        let layoutID: String
        if case .gameLayout(_, let requestedID) = next {
          layoutID = requestedID
        } else {
          guard
            let initial = bundle.choices.first(where: { $0.isDefault && $0.issues.isEmpty })
              ?? bundle.choices.first(where: { $0.issues.isEmpty })
          else {
            throw TouchLayoutError.invalid("布局包中暂无可用布局")
          }
          layoutID = initial.id
        }
        let snapshot = try await bundle.preview(layoutID: layoutID)
        prepared = try await PreparedTouchLayout.prepare(snapshot)
        guard prepared.layout.report.isSupported else {
          throw TouchLayoutError.invalid("此布局的操作能力等待适配")
        }
        appliedSelection = .gameLayout(bundleID: bundleID, layoutID: layoutID)
        resourceRevision = bundle.version + ":" + layoutID
      }
      try Task.checkCancellation()
      guard request == selectionRevision, generation == sessionGeneration,
        !next.isGameSelection || catalogRevision == cloudCatalogRevision,
        item.map({ library.isAvailable($0.id, revision: $0.revision) }) ?? true
      else { return false }
      savedRevision = resourceRevision ?? item?.revision
      commit(prepared.layout, assets: prepared.assets, selection: appliedSelection)
      if streamTarget == .cloud {
        if appliedSelection.isGameSelection { recentGameSelection = appliedSelection }
      } else {
        rememberCustomSelection(effectiveSelection(preferences: preferences))
      }
      pendingSelection = nil
      trace(
        "touchLayoutApplied", source: next.source, revision: request, startedAt: startedAt)
      if let item { await library.markUsed(item.id) }
      return request == selectionRevision && generation == sessionGeneration
    } catch {
      guard request == selectionRevision, generation == sessionGeneration,
        !next.isGameSelection || catalogRevision == cloudCatalogRevision
      else {
        return false
      }
      pendingSelection = nil
      if !Task.isCancelled {
        loadError = CloudLibraryDiagnostics.safeError(error)
        if case .gameBundle(let id) = next, cloudDownloadedBundles[id] == nil {
          cloudDownloadFailures[id] = SectionRequestFailure(error: error)
        }
        trace(
          "touchLayoutPrepareFailed", source: next.source, revision: request,
          startedAt: startedAt)
      }
      return false
    }
  }

  private func loadCloudBundle(
    _ id: String, request: UInt64, generation: UInt64?, catalogRevision: UInt64
  ) async throws -> any OfficialTouchBundleProviding {
    guard streamTarget == .cloud, cloudCatalog?.bundles.contains(where: { $0.id == id }) == true,
      let download = cloudDownload
    else { throw TouchLayoutError.invalid("当前游戏的布局来源暂缺") }
    if let cached = cloudDownloadedBundles[id] { return cached }
    let bundle = try await download(id)
    try Task.checkCancellation()
    guard request == selectionRevision, generation == sessionGeneration,
      catalogRevision == cloudCatalogRevision
    else { throw StreamingRuntimeError.superseded }
    cloudDownloadedBundles[id] = bundle
    cloudDownloadFailures[id] = nil
    return bundle
  }

  func cancelPendingSelection() {
    selectionRevision &+= 1
    pendingSelection = nil
  }

  func prepareForDeletion(_ id: String) throws {
    if recentCustomSelection == .saved(id) {
      recentCustomSelection = nil
      onRecentSelectionChanged?(nil)
    }
    if pendingSelection == .saved(id) { cancelPendingSelection() }
    guard selection == .saved(id) else { return }
    let prepared = try standardSnapshot()
    cancelPendingSelection()
    savedRevision = nil
    commit(prepared.layout, assets: .empty, selection: .standard)
  }

  func reconcile(preferences: MobilePreferences, library: SavedTouchLayoutsStore) {
    if let recentCustomSelection,
      isConfirmedUnavailable(recentCustomSelection, preferences: preferences, library: library)
    {
      self.recentCustomSelection = nil
      onRecentSelectionChanged?(nil)
    }
    let invalid = isConfirmedUnavailable(
      selection, preferences: preferences, library: library, revision: savedRevision)
    guard invalid, let standard else { return }
    cancelPendingSelection()
    savedRevision = nil
    commit(standard, assets: .empty, selection: .standard)
  }

  private func isConfirmedUnavailable(
    _ choice: TouchLayoutSelection, preferences: MobilePreferences,
    library: SavedTouchLayoutsStore, revision: String? = nil
  ) -> Bool {
    // 首次本机读取成功后，列表才具有删除与缺失判断的权威。
    switch choice {
    case .saved(let id): library.hasLoadedLayouts && !library.isAvailable(id, revision: revision)
    case .template(let id): !preferences.touchLayoutTemplates.contains { $0.id == id }
    case .gameBundle, .gameLayout: streamTarget != .cloud
    case .standard, .preferences: false
    }
  }

  private func standardSnapshot() throws -> PreparedTouchLayout {
    guard let standard else { throw TouchLayoutError.invalid("通用布局资源暂缺") }
    return PreparedTouchLayout(layout: standard, assets: .empty)
  }

  private func rememberCustomSelection(_ choice: TouchLayoutSelection) {
    guard let id = choice.persistentID else { return }
    recentCustomSelection = choice
    onRecentSelectionChanged?(id)
  }

  private func commit(
    _ layout: TouchLayout, assets: TouchLayoutAssets, selection: TouchLayoutSelection
  ) {
    // 资源已准备完成；旧触点、sticky、toggle 和 layer 经同一输出出口释放。
    input.cancelAll(reason: "touchLayoutReplaced")
    session?.releaseTouchesForLayoutChange()
    input.setConfiguration(nil)
    self.assets = assets
    input.load(layout)
    self.selection = selection
    loadError = nil
  }

  private func trace(_ event: String, source: String, revision: UInt64, startedAt: Date? = nil) {
    var payload: [String: IOSRuntimeTraceValue] = [
      "source": .string(source), "selectionRevision": .integer(Int64(clamping: revision)),
      "sessionGeneration": sessionGeneration.map { .integer(Int64(clamping: $0)) } ?? .null,
    ]
    if let startedAt {
      payload["elapsedMs"] = .integer(
        Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt)))
      payload["neutral"] = .bool(input.gamepad == .neutral && input.activeLayers.isEmpty)
    }
    IOSRuntimeTrace.event(
      domain: "touch-layout", event: event, payload: payload, dimension: .input,
      importance: .key)
  }
}
