import Foundation
import XCTest

@testable import XBXRC

final class HTTPBehaviorParityTests: XCTestCase {
  func testHostTerminalAndNetworkFactsHaveDistinctActions() {
    for kind in [XboxStartupFailureKind.hostRemotePlayUnavailable, .hostRegistrationRetryExhausted]
    {
      let error = XboxStreamingError.Startup(
        failure: XboxStartupFailure(
          kind: kind, phase: "creatingSession", retryable: true, statusCode: 503,
          failureCode: nil, networkErrorKind: nil))
      XCTAssertFalse(StreamingStartupDiagnostics.isRetryable(error, fallback: true))
      XCTAssertEqual(StreamingStartupDiagnostics.errorPayload(error)["retryable"], .bool(false))
    }
    let query = XboxQueryFailure(
      kind: .network, statusCode: nil, retriable: true,
      retryAfterSeconds: nil, networkErrorKind: "tls", serviceCode: nil, protocolErrorCode: nil)
    let failure = SectionRequestFailure(error: XboxBridgeError.Query(failure: query))
    XCTAssertEqual(failure.networkErrorKind, "tls")
    XCTAssertFalse(failure.allowsActivation())
    XCTAssertTrue(failure.allowsActivation(at: Date().addingTimeInterval(16)))
    XCTAssertFalse(SectionRequestFailure(kind: .authentication).allowsActivation())
  }

  func testMediaStatusSizeAndSignatureRemainDistinct() throws {
    let png = Data([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])
    func response(_ status: Int, mime: String = "image/png", retry: String = "30")
      -> HTTPURLResponse
    {
      HTTPURLResponse(
        url: URL(string: "https://fixture.invalid/media")!, statusCode: status,
        httpVersion: nil, headerFields: ["Content-Type": mime, "Retry-After": retry])!
    }
    XCTAssertEqual(
      try AlbumDownloadPolicy.validate(
        response: response(200), size: 8, kind: .screenshot, prefix: png), "png")
    for (status, kind) in [
      (403, SectionRequestFailure.Kind.accessRestricted), (429, .rateLimited),
      (503, .serviceUnavailable), (401, .mediaExpired),
    ] {
      do {
        _ = try AlbumDownloadPolicy.validate(
          response: response(status), size: 8, kind: .screenshot, prefix: png)
        XCTFail("HTTP error must retain its category")
      } catch {
        let failure = SectionRequestFailure(error: error)
        XCTAssertEqual(failure.kind, kind)
        XCTAssertEqual(failure.statusCode, UInt16(status))
        if status == 429 { XCTAssertGreaterThan(failure.retryAt!.timeIntervalSinceNow, 29) }
      }
    }
    for (size, prefix, kind) in [
      (Int64(0), png, SectionRequestFailure.Kind.mediaEmpty),
      (AlbumDownloadPolicy.maximumBytes + 1, png, .mediaTooLarge),
      (Int64(8), Data("<html>".utf8), .mediaInvalidFormat),
    ] {
      do {
        _ = try AlbumDownloadPolicy.validate(
          response: response(200), size: size, kind: .screenshot, prefix: prefix)
        XCTFail("Invalid media must fail")
      } catch { XCTAssertEqual(SectionRequestFailure(error: error).kind, kind) }
    }
    XCTAssertThrowsError(try AlbumDownloadPolicy.fileExtension(kind: .clip, prefix: png))
    XCTAssertTrue(SectionRequestFailure.isCancellation(URLError(.cancelled)))
  }

  func testDetailPersistsAcrossRestartAndHonorsFreshStaleAndExpiry() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let clock = ParityClock()
    let repository = CloudProductDetailRepository(root: root, now: { clock.date })
    let first = try await repository.load(productID: "GAME1", market: "US", language: "en-US") {
      Self.detail("GAME1")
    }
    let restarted = CloudProductDetailRepository(root: root, now: { clock.date })
    let fresh = try await restarted.load(productID: "game1", market: "us", language: "EN-us") {
      throw URLError(.cannotConnectToHost)
    }
    XCTAssertEqual(fresh.detail, first.detail)
    XCTAssertFalse(fresh.stale)
    clock.advance(25 * 60 * 60)
    let stale = try await restarted.load(productID: "GAME1", market: "US", language: "en-US") {
      throw URLError(.timedOut)
    }
    XCTAssertTrue(stale.stale)
    XCTAssertEqual(stale.updatedAt, first.updatedAt)
    clock.advance(7 * 24 * 60 * 60)
    do {
      _ = try await restarted.load(productID: "GAME1", market: "US", language: "en-US") {
        throw URLError(.timedOut)
      }
      XCTFail("Expired details must fail")
    } catch { XCTAssertEqual(SectionRequestFailure(error: error).kind, .timeout) }
  }

  func testDetailMergesInFlightAndBoundsDiskAcrossMarketsAndLanguages() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let repository = CloudProductDetailRepository(root: root, capacity: 2)
    let calls = ParityCounter()
    let fetch: @Sendable () async throws -> CloudProductDetails = {
      await calls.increment()
      try await Task.sleep(for: .milliseconds(30))
      return Self.detail("GAME1")
    }
    async let first = repository.load(
      productID: "GAME1", market: "US", language: "en-US", fetch: fetch)
    async let second = repository.load(
      productID: "GAME1", market: "US", language: "en-US", fetch: fetch)
    _ = try await (first, second)
    let count = await calls.value
    XCTAssertEqual(count, 1)
    _ = try await repository.load(productID: "GAME1", market: "JP", language: "en-US", fetch: fetch)
    _ = try await repository.load(productID: "GAME1", market: "JP", language: "ja-JP", fetch: fetch)
    let scopedCount = await calls.value
    XCTAssertEqual(scopedCount, 3)
    XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: root.path).count, 2)
  }

  func testDetailCancellationDropsLateResultAndAllowsNextAttempt() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let repository = CloudProductDetailRepository(root: root)
    let gate = ParityGate()
    let task = Task {
      try await repository.load(productID: "GAME1", market: "US", language: "en-US") {
        await gate.started()
        try await Task.sleep(for: .seconds(30))
        return Self.detail("GAME1")
      }
    }
    await gate.wait()
    task.cancel()
    do {
      _ = try await task.value
      XCTFail("Cancelled waiter must exit")
    } catch { XCTAssertTrue(SectionRequestFailure.isCancellation(error)) }
    let result = try await repository.load(productID: "GAME1", market: "US", language: "en-US") {
      Self.detail("GAME1")
    }
    XCTAssertFalse(result.stale)
  }

  private static func detail(_ id: String) -> CloudProductDetails {
    CloudProductDetails(
      productID: id, name: "Fixture", publisherName: "", description: "",
      tileURL: nil, posterURL: nil, heroURL: nil, categories: [], developer: "", releaseDate: "",
      rating: "", attributes: [], screenshots: [], platforms: [], languages: [], accessibility: [])
  }
}

private final class ParityClock: @unchecked Sendable {
  private let lock = NSLock()
  private var value = Date()
  var date: Date { lock.withLock { value } }
  func advance(_ seconds: TimeInterval) { lock.withLock { value.addTimeInterval(seconds) } }
}
private actor ParityCounter {
  private(set) var value = 0
  func increment() { value += 1 }
}
private actor ParityGate {
  private var ready = false
  private var continuation: CheckedContinuation<Void, Never>?
  func started() {
    ready = true
    continuation?.resume()
    continuation = nil
  }
  func wait() async {
    if ready { return }
    await withCheckedContinuation { continuation = $0 }
  }
}
