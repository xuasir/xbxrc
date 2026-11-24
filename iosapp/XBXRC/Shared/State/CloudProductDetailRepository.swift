import Foundation

struct CachedCloudProductDetail: Sendable {
  let detail: CloudProductDetails
  let updatedAt: Date
  let stale: Bool
}

/// 公开商品资料按市场/语言隔离；最后一个等待者取消时终止底层读取。
actor CloudProductDetailRepository {
  static let shared = CloudProductDetailRepository()
  static let freshDuration: TimeInterval = 24 * 60 * 60
  static let staleDuration: TimeInterval = 7 * 24 * 60 * 60
  private let root: URL
  private let capacity: Int
  private let now: @Sendable () -> Date
  private var memory: [String: DiskEntry] = [:]
  private var accesses: [String: Date] = [:]
  private var requests: [String: Request] = [:]
  private var queue: [String] = []
  private var active = 0

  private struct DiskEntry: Codable, Sendable {
    let schema: Int
    let key: String
    let detail: CloudProductDetails
    let updatedAt: Date
  }
  private struct Request {
    let id: UUID
    let productID: String
    let cached: DiskEntry?
    let fetch: @Sendable () async throws -> CloudProductDetails
    var waiters: [UUID: CheckedContinuation<CachedCloudProductDetail, any Error>]
    var task: Task<Void, Never>?
  }

  init(root: URL? = nil, capacity: Int = 30, now: @escaping @Sendable () -> Date = { .now }) {
    self.root =
      root
      ?? FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
      .appendingPathComponent("XBXRC/CloudProductDetails", isDirectory: true)
    self.capacity = max(1, capacity)
    self.now = now
  }

  func load(
    productID: String, market: String, language: String, force: Bool = false,
    fetch: @escaping @Sendable () async throws -> CloudProductDetails
  ) async throws -> CachedCloudProductDetail {
    try Task.checkCancellation()
    let id = productID.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
    let market = market.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
    let language = language.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    guard !id.isEmpty, id.count <= 32,
      id.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) }),
      !market.isEmpty, market.count <= 35, !language.isEmpty, language.count <= 35
    else { throw SectionRequestFailure(kind: .invalidResponse) }
    let key = CloudCatalogScope.fingerprint([id, market, language].joined(separator: "\u{1F}"))
    let waiter = UUID()
    return try await withTaskCancellationHandler {
      try await withCheckedThrowingContinuation { continuation in
        guard !Task.isCancelled else {
          continuation.resume(throwing: CancellationError())
          return
        }
        if var pending = requests[key] {
          pending.waiters[waiter] = continuation
          requests[key] = pending
          return
        }
        let cached = read(key: key, productID: id)
        if !force, let cached, now().timeIntervalSince(cached.updatedAt) < Self.freshDuration {
          continuation.resume(returning: value(cached, stale: false))
          return
        }
        guard requests.count < capacity else {
          continuation.resume(throwing: SectionRequestFailure(kind: .serviceUnavailable))
          return
        }
        requests[key] = Request(
          id: UUID(), productID: id, cached: cached, fetch: fetch, waiters: [waiter: continuation])
        queue.append(key)
        drain()
      }
    } onCancel: {
      Task { await self.cancel(key: key, waiter: waiter) }
    }
  }

  private func drain() {
    while active < 2, !queue.isEmpty {
      let key = queue.removeFirst()
      guard var request = requests[key], request.task == nil else { continue }
      active += 1
      let id = request.id
      let fetch = request.fetch
      request.task = Task {
        let result: Result<CloudProductDetails, any Error>
        do { result = .success(try await fetch()) } catch { result = .failure(error) }
        finish(key: key, id: id, result: result)
      }
      requests[key] = request
    }
  }

  private func finish(key: String, id: UUID, result: Result<CloudProductDetails, any Error>) {
    active -= 1
    defer { drain() }
    guard let request = requests[key], request.id == id else { return }
    requests[key] = nil
    let delivered: Result<CachedCloudProductDetail, any Error>
    switch result {
    case .success(let detail):
      if detail.productID.caseInsensitiveCompare(request.productID) != .orderedSame {
        delivered = .failure(SectionRequestFailure(kind: .invalidResponse))
      } else {
        let entry = DiskEntry(schema: 1, key: key, detail: detail, updatedAt: now())
        save(entry)
        delivered = .success(value(entry, stale: false))
      }
    case .failure(let error):
      if !SectionRequestFailure.isCancellation(error), let cached = request.cached,
        now().timeIntervalSince(cached.updatedAt) <= Self.staleDuration
      {
        delivered = .success(value(cached, stale: true))
      } else {
        delivered = .failure(error)
      }
    }
    for continuation in request.waiters.values { continuation.resume(with: delivered) }
  }

  private func cancel(key: String, waiter: UUID) {
    guard var request = requests[key],
      let continuation = request.waiters.removeValue(forKey: waiter)
    else { return }
    continuation.resume(throwing: CancellationError())
    if request.waiters.isEmpty {
      requests[key] = nil
      queue.removeAll { $0 == key }
      request.task?.cancel()
    } else {
      requests[key] = request
    }
  }

  private func value(_ entry: DiskEntry, stale: Bool) -> CachedCloudProductDetail {
    CachedCloudProductDetail(detail: entry.detail, updatedAt: entry.updatedAt, stale: stale)
  }

  private func read(key: String, productID: String) -> DiskEntry? {
    let url = root.appendingPathComponent(key + ".json")
    let cached: DiskEntry?
    if let entry = memory[key] {
      cached = entry
    } else if let size = try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize,
      size <= 4 * 1024 * 1024,
      let data = try? Data(contentsOf: url),
      let entry = try? JSONDecoder().decode(DiskEntry.self, from: data)
    {
      cached = entry
    } else {
      cached = nil
    }
    guard let cached, cached.schema == 1, cached.key == key,
      cached.detail.productID.caseInsensitiveCompare(productID) == .orderedSame,
      cached.updatedAt <= now().addingTimeInterval(300),
      now().timeIntervalSince(cached.updatedAt) <= Self.staleDuration
    else {
      memory[key] = nil
      accesses[key] = nil
      try? FileManager.default.removeItem(at: url)
      return nil
    }
    remember(cached)
    return cached
  }

  private func remember(_ entry: DiskEntry) {
    memory[entry.key] = entry
    accesses[entry.key] = now()
    while memory.count > capacity, let oldest = accesses.min(by: { $0.value < $1.value })?.key {
      memory[oldest] = nil
      accesses[oldest] = nil
    }
  }

  private func save(_ entry: DiskEntry) {
    remember(entry)
    do {
      let data = try JSONEncoder().encode(entry)
      guard data.count <= 4 * 1024 * 1024 else { return }
      try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
      try data.write(to: root.appendingPathComponent(entry.key + ".json"), options: .atomic)
      let files = try FileManager.default.contentsOfDirectory(
        at: root, includingPropertiesForKeys: [.contentModificationDateKey]
      )
      .filter { $0.pathExtension == "json" }.sorted {
        let first =
          (try? $0.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate)
          ?? .distantPast
        let second =
          (try? $1.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate)
          ?? .distantPast
        return first > second
      }
      for file in files.dropFirst(capacity) { try FileManager.default.removeItem(at: file) }
    } catch {
      // 磁盘缓存尽力写入，已成功取得的公开资料仍正常返回。
    }
  }
}
