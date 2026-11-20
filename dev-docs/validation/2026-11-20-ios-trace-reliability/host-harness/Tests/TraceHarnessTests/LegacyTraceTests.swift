import Foundation
import XCTest
@testable import TraceHarness
final class LegacyTraceTests: XCTestCase, @unchecked Sendable {
func testDiagnosticCaptureWorksWithBackgroundOffAndKeepsRedaction() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let writer = IOSRuntimeTraceWriter(rootDirectory: root, profile: .off)
        try await writer.startCapture()
        let active = await writer.diagnosticRecords()
        XCTAssertEqual(active.count, 1)
        XCTAssertTrue(active[0].isActive)
        XCTAssertEqual(writer.currentProfile, .off)
        writer.record(
            IOSRuntimeTraceDraft(
                category: .event, domain: "settings", event: "testCapture",
                payload: [
                    "refreshToken": .string("private-token"),
                    "serverCredential": .string("private-secret"),
                ],
                dimension: .core, importance: .essential, operationID: nil))
        _ = await writer.stopCapture()
        let records = await writer.diagnosticRecords()
        XCTAssertFalse(records[0].isActive)
        XCTAssertNotNil(records[0].endedAt)
        let export = try await writer.diagnosticExport(records[0])
        defer { try? FileManager.default.removeItem(at: export) }
        let content = try String(contentsOf: export, encoding: .utf8)
        XCTAssertTrue(content.contains("testCapture"))
        XCTAssertFalse(content.contains("private-token"))
        XCTAssertFalse(content.contains("private-secret"))
        let repeatedStop = await writer.stopCapture()
        XCTAssertNil(repeatedStop)
        await writer.clearFiles()
        let cleared = await writer.diagnosticRecords()
        XCTAssertTrue(cleared.isEmpty)
    }
func testDiagnosticCaptureRetentionKeepsThreeRecords() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let writer = IOSRuntimeTraceWriter(rootDirectory: root, profile: .production)
        for _ in 0..<5 {
            try await writer.startCapture()
            _ = await writer.stopCapture()
        }
        let records = await writer.diagnosticRecords()
        XCTAssertEqual(records.count, 3)
    }
func testTraceWriterCanReleaseLastReferenceOnItsIOQueue() async {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let queue = DispatchQueue(label: "XBXRC.TraceWriterDeinitTest")
        queue.suspend()
        var writer: IOSRuntimeTraceWriter? = IOSRuntimeTraceWriter(
            rootDirectory: root, profile: .off, queue: queue)
        weak var releasedWriter = writer
        queue.async { [retainedWriter = writer!] in
            withExtendedLifetime(retainedWriter) {}
        }
        writer = nil
        queue.resume()
        await withCheckedContinuation { continuation in
            queue.async { continuation.resume() }
        }
        XCTAssertNil(releasedWriter, "写入队列上的最后一份引用应能完成析构")
    }
func testFailureDiagnosticsSaveRedactedSnapshotWithoutManualCapture() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let writer = IOSRuntimeTraceWriter(rootDirectory: root, profile: .production)
        writer.record(
            IOSRuntimeTraceDraft(
                category: .event, domain: "ios-auth", event: "authSignInFailed",
                payload: ["refreshToken": .string("secret-value")], dimension: .core,
                importance: .key, operationID: nil))
        await writer.flush()
        let records = await writer.diagnosticRecords()
        XCTAssertEqual(records.count, 1)
        XCTAssertEqual(records[0].operation, "ios-auth")
        let export = try await writer.diagnosticExport(records[0])
        defer { try? FileManager.default.removeItem(at: export) }
        let text = try String(contentsOf: export, encoding: .utf8)
        XCTAssertTrue(text.contains("authSignInFailed"))
        XCTAssertFalse(text.contains("secret-value"))
    }
func testLightweightDiagnosticsRetainsFailuresAndManualCaptureDetail() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let writer = IOSRuntimeTraceWriter(rootDirectory: root, profile: .dev)
        let previous = UserDefaults.standard.bool(forKey: "ios.diagnostics.lightweight")
        defer { writer.setLightweight(previous) }
        writer.setLightweight(true)
        let draft = IOSRuntimeTraceDraft(
            category: .snapshot, domain: "settings", event: "detailSnapshot",
            payload: [:], dimension: .core, importance: .debug, operationID: nil)
        writer.record(draft)
        await writer.flush()
        let files = await writer.traceFiles()
        let before = try String(contentsOf: XCTUnwrap(files.last), encoding: .utf8)
        XCTAssertFalse(before.contains("detailSnapshot"))
        try await writer.startCapture()
        writer.record(draft)
        _ = await writer.stopCapture()
        let records = await writer.diagnosticRecords()
        let export = try await writer.diagnosticExport(XCTUnwrap(records.first))
        defer { try? FileManager.default.removeItem(at: export) }
        let captured = try String(contentsOf: export, encoding: .utf8)
        XCTAssertTrue(captured.contains("detailSnapshot"))
    }
func testIOSRuntimeTraceWriterEmitsSchemaV3AndRedactsPayload() async throws {
        let rootDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: rootDirectory) }
        let writer = IOSRuntimeTraceWriter(
            rootDirectory: rootDirectory,
            profile: .dev,
            launchSessionID: "launch-session"
        )

        writer.record(
            IOSRuntimeTraceDraft(
                category: .event,
                domain: "cloud-library",
                event: "catalogRefreshStarted",
                payload: [
                    "generation": .integer(3),
                    "refreshToken": .string("secret-refresh-token"),
                    "hasRefreshToken": .bool(true),
                    "accountID": .string("123456789"),
                    "message": .string("request https://example.invalid/path?token=secret"),
                ],
                dimension: .network,
                importance: .key,
                operationID: "operation-1"
            )
        )
        writer.record(
            IOSRuntimeTraceDraft(
                category: .state,
                domain: "cloud-library",
                event: "catalogRefreshCommitted",
                payload: ["games": .integer(1_888)],
                dimension: .core,
                importance: .essential,
                operationID: "operation-1"
            )
        )

        await writer.flush()
        let envelopes = try await traceEnvelopes(from: writer)
        let events = envelopes.filter { $0.domain == "cloud-library" }

        XCTAssertEqual(events.count, 2)
        XCTAssertEqual(events.map(\.seq), events.map(\.seq).sorted())
        XCTAssertTrue(events.allSatisfy { $0.schemaVersion == 3 })
        XCTAssertTrue(events.allSatisfy { $0.sessionId == "launch-session" })
        XCTAssertEqual(events.first?.payload["refreshToken"], .string("<redacted>"))
        XCTAssertEqual(events.first?.payload["hasRefreshToken"], .bool(true))
        XCTAssertEqual(events.first?.payload["accountID"], .string("<redacted>"))
        XCTAssertEqual(events.first?.payload["message"], .string("request <url>"))
        XCTAssertEqual(events.first?.payload["operationId"], .string("operation-1"))
        XCTAssertEqual(events.first?.payload["platform"], .string("ios"))
    }
func testIOSRuntimeTraceProductionFiltersDebugRows() async throws {
        let rootDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: rootDirectory) }
        let writer = IOSRuntimeTraceWriter(
            rootDirectory: rootDirectory,
            profile: .production
        )

        writer.record(
            IOSRuntimeTraceDraft(
                category: .event,
                domain: "test",
                event: "debugRow",
                payload: [:],
                dimension: .core,
                importance: .debug,
                operationID: nil
            )
        )
        writer.record(
            IOSRuntimeTraceDraft(
                category: .state,
                domain: "test",
                event: "keyRow",
                payload: [:],
                dimension: .core,
                importance: .key,
                operationID: nil
            )
        )

        await writer.flush()
        let envelopes = try await traceEnvelopes(from: writer)

        XCTAssertFalse(envelopes.contains { $0.event == "debugRow" })
        XCTAssertTrue(envelopes.contains { $0.event == "keyRow" })
    }
func testIOSRuntimeTraceProfileBudgetsStayBounded() {
        XCTAssertEqual(
            IOSRuntimeTracePolicy.budget(for: .production),
            IOSRuntimeTraceBudget(maxFileBytes: 8 * 1_024 * 1_024, maxFiles: 4)
        )
        XCTAssertEqual(
            IOSRuntimeTracePolicy.budget(for: .dev),
            IOSRuntimeTraceBudget(maxFileBytes: 32 * 1_024 * 1_024, maxFiles: 6)
        )
        XCTAssertEqual(
            IOSRuntimeTracePolicy.budget(for: .off),
            IOSRuntimeTraceBudget(maxFileBytes: 0, maxFiles: 0)
        )
    }
func testIOSRuntimeTraceRotatesAndPrunesFiles() async throws {
        let rootDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: rootDirectory) }
        let writer = IOSRuntimeTraceWriter(
            rootDirectory: rootDirectory,
            profile: .dev,
            budgetOverride: IOSRuntimeTraceBudget(maxFileBytes: 1_800, maxFiles: 2)
        )

        for index in 0..<24 {
            writer.record(
                IOSRuntimeTraceDraft(
                    category: .snapshot,
                    domain: "rotation-test",
                    event: "sample",
                    payload: [
                        "index": .integer(Int64(index)),
                        "content": .string(String(repeating: "x", count: 160)),
                    ],
                    dimension: .core,
                    importance: .key,
                    operationID: nil
                )
            )
        }

        await writer.flush()
        let files = await writer.traceFiles()
        let envelopes = try await traceEnvelopes(from: writer)
        let fileSizes = try files.map { file in
            try XCTUnwrap(
                file.resourceValues(forKeys: [.fileSizeKey]).fileSize
            )
        }

        XCTAssertEqual(files.count, 2)
        XCTAssertTrue(fileSizes.allSatisfy { $0 <= 1_800 })
        XCTAssertFalse(envelopes.isEmpty)
        XCTAssertTrue(envelopes.contains { $0.event == "sample" })
        XCTAssertEqual(envelopes.map(\.seq), envelopes.map(\.seq).sorted())
        XCTAssertEqual(Set(envelopes.map(\.seq)).count, envelopes.count)
        XCTAssertTrue(envelopes.contains { $0.event == "fileOpened" })
    }
private func traceEnvelopes(
        from writer: IOSRuntimeTraceWriter
    ) async throws -> [IOSRuntimeTraceEnvelope] {
        let decoder = JSONDecoder()
        let files = await writer.traceFiles()
        return try files.flatMap { file in
            try String(contentsOf: file, encoding: .utf8)
                .split(separator: "\n")
                .map { line in
                    try decoder.decode(
                        IOSRuntimeTraceEnvelope.self,
                        from: Data(line.utf8)
                    )
                }
        }
    }
}
