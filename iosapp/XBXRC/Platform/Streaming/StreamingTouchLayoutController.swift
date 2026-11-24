import Combine
import Foundation

enum TouchLayoutSelection: Equatable, Sendable {
  case preferences
  case standard
  case template(String)
  case saved(String)

  var persistentID: String? {
    switch self {
    case .template(let id): "template:" + id
    case .saved(let id): id
    case .standard, .preferences: nil
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
  private let standard: TouchLayout?
  private var sessionGeneration: UInt64?
  private var selectionRevision: UInt64 = 0
  private var savedRevision: String?
  private weak var session: StreamInputSession?
  private var suppressionSubscription: AnyCancellable?

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

  func beginSession(generation: UInt64, preferences: MobilePreferences = .init()) {
    sessionGeneration = generation
    cancelPendingSelection()
    if let standard { commit(standard, assets: .empty, selection: .preferences) }
    rememberCustomSelection(effectiveSelection(preferences: preferences))
  }

  func endSession() {
    sessionGeneration = nil
    cancelPendingSelection()
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
    selectionRevision &+= 1
    let request = selectionRevision
    let generation = sessionGeneration
    guard generation != nil else { return false }
    pendingSelection = next
    loadError = nil
    let startedAt = Date()
    trace("touchLayoutPrepareStarted", source: next.source, revision: request)
    do {
      let prepared: PreparedTouchLayout
      var item: XboxSavedTouchLayout?
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
      }
      try Task.checkCancellation()
      guard request == selectionRevision, generation == sessionGeneration,
        item.map({ library.isAvailable($0.id, revision: $0.revision) }) ?? true
      else { return false }
      savedRevision = item?.revision
      commit(prepared.layout, assets: prepared.assets, selection: next)
      rememberCustomSelection(effectiveSelection(preferences: preferences))
      pendingSelection = nil
      trace(
        "touchLayoutApplied", source: next.source, revision: request, startedAt: startedAt)
      if let item { await library.markUsed(item.id) }
      return request == selectionRevision && generation == sessionGeneration
    } catch {
      guard request == selectionRevision, generation == sessionGeneration else {
        return false
      }
      pendingSelection = nil
      if !Task.isCancelled {
        loadError = CloudLibraryDiagnostics.safeError(error)
        trace(
          "touchLayoutPrepareFailed", source: next.source, revision: request,
          startedAt: startedAt)
      }
      return false
    }
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
