import Foundation

enum AlbumDownloadFailure: Error, Sendable {
  case expired, tooLarge, empty, invalidFormat, storage
  case http(status: Int, retryAfter: TimeInterval?)
}

enum AlbumDownloadPolicy {
  static let maximumBytes: Int64 = 2 * 1024 * 1024 * 1024

  static func fileExtension(kind: XboxAlbumKind, prefix: Data) throws -> String {
    let bytes = [UInt8](prefix)
    let ftyp = bytes.count >= 12 && Data(bytes[4..<8]) == Data("ftyp".utf8)
    let brand = bytes.count >= 12 ? String(decoding: bytes[8..<12], as: UTF8.self) : ""
    let imageBrand = ["avif", "avis", "heic", "heix", "mif1"].contains(brand)
    if kind == .clip {
      guard ftyp && !imageBrand else { throw AlbumDownloadFailure.invalidFormat }
      return "mp4"
    }
    if prefix.starts(with: [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]) { return "png" }
    if prefix.starts(with: [0xff, 0xd8, 0xff]) { return "jpg" }
    if prefix.starts(with: Data("GIF87a".utf8)) || prefix.starts(with: Data("GIF89a".utf8)) {
      return "gif"
    }
    if prefix.starts(with: Data("RIFF".utf8)), bytes.count >= 12,
      Data(bytes[8..<12]) == Data("WEBP".utf8)
    {
      return "webp"
    }
    if ftyp && imageBrand { return brand.hasPrefix("avi") ? "avif" : "heic" }
    throw AlbumDownloadFailure.invalidFormat
  }

  static func validate(response: HTTPURLResponse, size: Int64, kind: XboxAlbumKind, prefix: Data)
    throws -> String
  {
    guard (200..<300).contains(response.statusCode) else {
      let raw = response.value(forHTTPHeaderField: "Retry-After")
      let seconds = raw.flatMap(TimeInterval.init).flatMap {
        $0.isFinite && $0 >= 0 ? min($0, 86_400) : nil
      }
      throw AlbumDownloadFailure.http(status: response.statusCode, retryAfter: seconds)
    }
    guard size > 0 else { throw AlbumDownloadFailure.empty }
    guard size <= maximumBytes, response.expectedContentLength <= maximumBytes else {
      throw AlbumDownloadFailure.tooLarge
    }
    if let mime = response.mimeType, mime != "application/octet-stream",
      !(kind == .clip ? mime.hasPrefix("video/") : mime.hasPrefix("image/"))
    {
      throw AlbumDownloadFailure.invalidFormat
    }
    return try fileExtension(kind: kind, prefix: prefix)
  }
}

/// 原生下载只在校验成功后发布文件；失败和取消统一结束 session 并清理留存文件。
final class AlbumFileDownload: NSObject, URLSessionDownloadDelegate, @unchecked Sendable {
  private let lock = NSLock()
  private let kind: XboxAlbumKind
  private let onProgress: @MainActor @Sendable (Int64, Int64) -> Void
  private var continuation: CheckedContinuation<(URL, String), any Error>?
  private var task: URLSessionDownloadTask?
  private var session: URLSession?
  private var failure: (any Error)?

  init(kind: XboxAlbumKind, onProgress: @escaping @MainActor @Sendable (Int64, Int64) -> Void) {
    self.kind = kind
    self.onProgress = onProgress
  }

  func download(from url: URL) async throws -> (URL, String) {
    guard url.scheme?.lowercased() == "https", url.host != nil,
      url.user == nil, url.password == nil
    else { throw AlbumDownloadFailure.expired }
    try Task.checkCancellation()
    return try await withTaskCancellationHandler {
      try await withCheckedThrowingContinuation { continuation in
        lock.lock()
        if Task.isCancelled {
          lock.unlock()
          continuation.resume(throwing: CancellationError())
          return
        }
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 300
        config.timeoutIntervalForResource = 300
        let session = URLSession(configuration: config, delegate: self, delegateQueue: nil)
        let task = session.downloadTask(with: url)
        self.continuation = continuation
        self.session = session
        self.task = task
        lock.unlock()
        task.resume()
      }
    } onCancel: {
      self.cancel(CancellationError())
    }
  }

  private func cancel(_ error: any Error) {
    lock.lock()
    if failure == nil { failure = error }
    let task = task
    lock.unlock()
    task?.cancel()
  }

  func urlSession(
    _ session: URLSession, downloadTask: URLSessionDownloadTask,
    didWriteData bytesWritten: Int64, totalBytesWritten: Int64, totalBytesExpectedToWrite: Int64
  ) {
    if totalBytesWritten > AlbumDownloadPolicy.maximumBytes
      || totalBytesExpectedToWrite > AlbumDownloadPolicy.maximumBytes
    {
      cancel(AlbumDownloadFailure.tooLarge)
    }
    Task { @MainActor in onProgress(totalBytesWritten, totalBytesExpectedToWrite) }
  }

  func urlSession(
    _ session: URLSession, downloadTask: URLSessionDownloadTask,
    didFinishDownloadingTo location: URL
  ) {
    do {
      guard let response = downloadTask.response as? HTTPURLResponse else {
        throw AlbumDownloadFailure.invalidFormat
      }
      let attributes = try FileManager.default.attributesOfItem(atPath: location.path)
      let size = (attributes[.size] as? NSNumber)?.int64Value ?? 0
      let handle = try FileHandle(forReadingFrom: location)
      defer { try? handle.close() }
      let suffix = try AlbumDownloadPolicy.validate(
        response: response, size: size, kind: kind, prefix: try handle.read(upToCount: 32) ?? Data()
      )
      let retained = FileManager.default.temporaryDirectory.appendingPathComponent(
        "xbxrc-media-\(UUID().uuidString).\(suffix)")
      try FileManager.default.moveItem(at: location, to: retained)
      finish(.success((retained, suffix)))
    } catch { finish(.failure(error)) }
  }

  func urlSession(
    _ session: URLSession, task: URLSessionTask, didCompleteWithError error: (any Error)?
  ) {
    if let error { finish(.failure(error)) }
  }

  func urlSession(
    _ session: URLSession, task: URLSessionTask,
    willPerformHTTPRedirection response: HTTPURLResponse,
    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void
  ) {
    guard let url = request.url, url.scheme?.lowercased() == "https",
      url.host != nil, url.user == nil, url.password == nil
    else {
      cancel(AlbumDownloadFailure.expired)
      completionHandler(nil)
      return
    }
    completionHandler(request)
  }

  private func finish(_ result: Result<(URL, String), any Error>) {
    lock.lock()
    let continuation = continuation
    self.continuation = nil
    let session = session
    self.session = nil
    task = nil
    let failure = failure
    lock.unlock()
    session?.finishTasksAndInvalidate()
    if continuation == nil || failure != nil, case .success(let output) = result {
      try? FileManager.default.removeItem(at: output.0)
    }
    if let failure {
      continuation?.resume(throwing: failure)
    } else {
      continuation?.resume(with: result)
    }
  }
}
