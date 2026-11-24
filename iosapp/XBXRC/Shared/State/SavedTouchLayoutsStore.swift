import Combine
import Foundation

@MainActor
final class SavedTouchLayoutsStore: ObservableObject {
  @Published private(set) var layouts: [XboxSavedTouchLayout] = []
  @Published private(set) var isLoading = false
  @Published private(set) var hasLoadedLayouts = false
  @Published private(set) var errorMessage: String?
  @Published private(set) var deletingIDs: Set<String> = []
  @Published private(set) var isSaving = false
  let client: any XboxTouchLayoutClient
  private var revision: UInt64 = 0

  init(client: any XboxTouchLayoutClient = RustXboxTouchLayoutClient()) { self.client = client }

  var groups: [SavedTouchLayoutGroup] {
    Dictionary(grouping: layouts, by: \.xboxTitleId).map { titleID, items in
      SavedTouchLayoutGroup(
        id: titleID, name: items.first?.gameName ?? "",
        layouts: items.sorted {
          $0.lastUsedAtMs == $1.lastUsedAtMs
            ? $0.layoutName.localizedStandardCompare($1.layoutName) == .orderedAscending
            : $0.lastUsedAtMs > $1.lastUsedAtMs
        })
    }.sorted {
      $0.lastUsedAt == $1.lastUsedAt
        ? $0.name.localizedStandardCompare($1.name) == .orderedAscending
        : $0.lastUsedAt > $1.lastUsedAt
    }
  }

  func reload() async {
    revision &+= 1
    let request = revision
    isLoading = true
    errorMessage = nil
    defer { if request == revision { isLoading = false } }
    do {
      let result = try await client.savedLayouts()
      try Task.checkCancellation()
      guard request == revision else { return }
      layouts = result
      hasLoadedLayouts = true
    } catch {
      guard request == revision, !Task.isCancelled else { return }
      errorMessage = CloudLibraryDiagnostics.safeError(error)
    }
  }

  func save(bundle: any OfficialTouchBundleProviding, prepared: PreparedTouchLayout) async throws {
    guard prepared.layout.report.isSupported,
      bundle.choices.contains(where: { $0.id == prepared.id && $0.issues.isEmpty }), !isSaving
    else { throw TouchLayoutError.invalid("此布局的操作能力等待适配") }
    try Task.checkCancellation()
    isSaving = true
    revision &+= 1
    let request = revision
    defer { isSaving = false }
    let result = try await client.save(bundle: bundle, layoutID: prepared.id)
    if request == revision {
      layouts = result
      hasLoadedLayouts = true
      isLoading = false
      errorMessage = nil
    } else {
      await reload()
    }
  }

  func isAvailable(_ id: String, revision: String? = nil) -> Bool {
    layouts.contains { $0.id == id && (revision == nil || $0.revision == revision) }
      && !deletingIDs.contains(id)
  }

  func load(_ item: XboxSavedTouchLayout) async throws -> PreparedTouchLayout {
    guard isAvailable(item.id, revision: item.revision) else {
      throw TouchLayoutError.invalid("已保存布局已更新或正在删除")
    }
    let snapshot = try await client.load(id: item.id)
    let result = try await PreparedTouchLayout.prepare(snapshot)
    try Task.checkCancellation()
    guard isAvailable(item.id, revision: item.revision), result.id == item.id,
      result.layout.report.isSupported
    else { throw TouchLayoutError.invalid("已保存布局需要重新适配") }
    return result
  }

  func delete(_ item: XboxSavedTouchLayout, controller: StreamingTouchLayoutController)
    async throws
  {
    guard deletingIDs.insert(item.id).inserted else { return }
    defer { deletingIDs.remove(item.id) }
    // 先释放当前布局，提交通用布局后再回收包资源。
    try controller.prepareForDeletion(item.id)
    revision &+= 1
    let request = revision
    let result: [XboxSavedTouchLayout]
    do {
      result = try await client.delete(id: item.id)
    } catch {
      // 索引可能已提交而资源回收失败；重读列表同步状态并重试回收。
      await reload()
      throw error
    }
    if request == revision {
      layouts = result
      hasLoadedLayouts = true
      isLoading = false
      errorMessage = nil
    } else {
      await reload()
    }
  }

  func markUsed(_ id: String) async {
    do {
      try await client.markUsed(id: id)
      if let index = layouts.firstIndex(where: { $0.id == id }) {
        layouts[index].lastUsedAtMs = UInt64(Date().timeIntervalSince1970 * 1000)
      }
    } catch {
      IOSRuntimeTrace.event(
        domain: "touch-layout", event: "savedTouchLayoutUsageFailed", payload: [:],
        dimension: .input, importance: .debug)
    }
  }
}
