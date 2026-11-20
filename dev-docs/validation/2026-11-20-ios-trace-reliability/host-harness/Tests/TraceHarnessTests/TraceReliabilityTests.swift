import Foundation
import XCTest
@testable import TraceHarness
final class TraceReliabilityTests: XCTestCase, @unchecked Sendable {
    private func root() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(
            "trace-reliability-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }

    private func draft(
        _ event: String, importance: IOSRuntimeTraceImportance = .key,
        category: IOSRuntimeTraceCategory = .state, payload: [String: IOSRuntimeTraceValue] = [:]
    ) -> IOSRuntimeTraceDraft {
        IOSRuntimeTraceDraft(
            category: category, domain: "ios-streaming", event: event,
            payload: payload, dimension: .core, importance: importance, operationID: "trace-test")
    }

    private func rows(_ writer: IOSRuntimeTraceWriter) async throws -> [IOSRuntimeTraceEnvelope] {
        var rows: [IOSRuntimeTraceEnvelope] = []
        for file in await writer.traceFiles() {
            let content = try String(contentsOf: file, encoding: .utf8)
            for line in content.split(separator: "\n") {
                rows.append(
                    try JSONDecoder().decode(IOSRuntimeTraceEnvelope.self, from: Data(line.utf8)))
            }
        }
        return rows
    }

    func testUnavailableStorageHasBoundedMemoryExplicitExportErrorAndRecovery() async throws {
        let parent = try root()
        let blocked = parent.appendingPathComponent("blocked")
        try Data("ordinary file".utf8).write(to: blocked)
        let writer = IOSRuntimeTraceWriter(rootDirectory: blocked, profile: .dev)
        for _ in 0..<10 { writer.record(draft("lostEvent")) }
        await writer.flush()
        let failed = await writer.status()
        XCTAssertNotNil(failed.storageFailure)
        XCTAssertEqual(failed.bufferedBytes, 0)
        XCTAssertEqual(failed.storageDroppedRows, 10)
        let activeURL = await writer.currentFileURL()
        XCTAssertNil(activeURL)
        do {
            _ = try await writer.prepareExport(allFiles: true)
            XCTFail("存储故障应返回明确错误")
        } catch { XCTAssertTrue(error is IOSRuntimeTraceStorageFailure) }
        try FileManager.default.removeItem(at: blocked)
        try FileManager.default.createDirectory(at: blocked, withIntermediateDirectories: true)
        await writer.flush()
        writer.record(draft("recoveredEvent"))
        let recovered = try await rows(writer)
        XCTAssertTrue(recovered.contains { $0.event == "traceStorageRecovered" })
        XCTAssertTrue(recovered.contains { $0.event == "recoveredEvent" })
        XCTAssertTrue(recovered.contains { $0.payload["storageDroppedRows"] == .integer(10) })
    }

    func testWriteFailureCountsLostTailAndRecoversWithoutRepeatingPrefix() async throws {
        let writer = IOSRuntimeTraceWriter(rootDirectory: try root(), profile: .dev)
        writer.record(draft("persistedPrefix"))
        await writer.flush()
        let handle = try XCTUnwrap(
            Mirror(reflecting: writer).children.first { $0.label == "fileHandle" }?.value
                as? FileHandle)
        try handle.close()
        for _ in 0..<4 { writer.record(draft("lostTail")) }
        await writer.flush()
        let failed = await writer.status()
        XCTAssertEqual(failed.storageFailure?.stage, "write")
        XCTAssertEqual(failed.storageDroppedRows, 4)
        XCTAssertEqual(failed.bufferedBytes, 0)
        await writer.flush()
        writer.record(draft("afterRecovery"))
        let exported = try await rows(writer)
        XCTAssertEqual(exported.filter { $0.event == "persistedPrefix" }.count, 1)
        XCTAssertEqual(exported.filter { $0.event == "afterRecovery" }.count, 1)
        XCTAssertEqual(exported.filter { $0.event == "lostTail" }.count, 0)
        XCTAssertEqual(Set(exported.map(\.seq)).count, exported.count)
    }

    func testQueueHasHardCapacityReservedAnchorsAndExactDropSummary() async throws {
        let q = DispatchQueue(label: "trace.pressure.regression")
        q.suspend()
        let writer = IOSRuntimeTraceWriter(rootDirectory: try root(), profile: .dev, queue: q)
        for _ in 0..<5000 { writer.record(draft("keyEvent")) }
        for _ in 0..<50 { writer.record(draft("debugEvent", importance: .debug)) }
        for _ in 0..<200 { writer.record(draft("essentialEvent", importance: .essential)) }
        q.resume()
        await writer.flush()
        let status = await writer.status()
        XCTAssertEqual(status.highWaterRows, IOSRuntimeTracePolicy.pendingRowLimit)
        let exported = try await rows(writer)
        XCTAssertEqual(exported.filter { $0.event == "keyEvent" }.count, 3968)
        XCTAssertEqual(exported.filter { $0.event == "essentialEvent" }.count, 128)
        let notices = exported.filter { $0.event == "traceBudgetNotice" }
        func sum(_ field: String) -> Int64 {
            notices.reduce(0) { sum, row in
                if case .integer(let n) = row.payload[field] { sum + n } else { sum }
            }
        }
        XCTAssertEqual(sum("keyDropped"), 1032)
        XCTAssertEqual(sum("essentialDropped"), 72)
        XCTAssertEqual(sum("debugDropped"), 50)
    }

    func testFlushIncludesRemainingDropsWithinNoticeInterval() async throws {
        let q = DispatchQueue(label: "trace.tail.regression")
        let writer = IOSRuntimeTraceWriter(rootDirectory: try root(), profile: .dev, queue: q)
        for _ in 0..<2 {
            q.suspend()
            for _ in 0..<2100 { writer.record(draft("debugEvent", importance: .debug)) }
            q.resume()
            await writer.flush()
        }
        let notices = try await rows(writer).filter { $0.event == "traceBudgetNotice" }
        let lost = notices.reduce(Int64(0)) { sum, row in
            if case .integer(let n) = row.payload["debugDropped"] { sum + n } else { sum }
        }
        XCTAssertEqual(lost, 104)
        XCTAssertTrue(notices.contains { $0.payload["finalSummary"] == .bool(true) })
    }

    func testProfileDisableFlushesAcceptedEvidenceAndDropSummary() async throws {
        let previous = UserDefaults.standard.object(forKey: IOSRuntimeTracePolicy.profileDefaultsKey)
        defer { UserDefaults.standard.set(previous, forKey: IOSRuntimeTracePolicy.profileDefaultsKey) }
        let q = DispatchQueue(label: "trace.profile.regression")
        q.suspend()
        let writer = IOSRuntimeTraceWriter(rootDirectory: try root(), profile: .dev, queue: q)
        for _ in 0..<2100 { writer.record(draft("debugEvent", importance: .debug)) }
        writer.setProfile(.off)
        q.resume()
        await writer.flush()
        let exported = try await rows(writer)
        XCTAssertEqual(exported.filter { $0.event == "debugEvent" }.count, 2048)
        XCTAssertTrue(exported.contains { $0.payload["debugDropped"] == .integer(52) })
        XCTAssertEqual(writer.currentProfile, .off)
    }

    func testTimestampUsesEventSourceWhileQueueDelayIsSeparate() async throws {
        let q = DispatchQueue(label: "trace.timestamp.regression")
        q.suspend()
        let writer = IOSRuntimeTraceWriter(rootDirectory: try root(), profile: .dev, queue: q)
        let event = draft("delayedEvent")
        writer.record(event)
        try await Task.sleep(for: .milliseconds(60))
        q.resume()
        let exported = try await rows(writer)
        let row = try XCTUnwrap(exported.first { $0.event == "delayedEvent" })
        XCTAssertEqual(row.tsMs, event.tsMs)
        if case .double(let delay) = row.payload["writerQueueDelayMs"] {
            XCTAssertGreaterThanOrEqual(delay, 50)
        } else {
            XCTFail("应有独立队列等待时间")
        }
    }

    func testOversizedRowIsCountedAndSubsequentEvidencePersists() async throws {
        let writer = IOSRuntimeTraceWriter(rootDirectory: try root(), profile: .dev)
        writer.record(
            draft("oversized", payload: ["text": .string(String(repeating: "x", count: 100_000))]))
        writer.record(draft("afterOversized"))
        let exported = try await rows(writer)
        XCTAssertFalse(exported.contains { $0.event == "oversized" })
        XCTAssertTrue(exported.contains { $0.event == "afterOversized" })
        let status = await writer.status()
        XCTAssertEqual(status.storageDroppedRows, 1)
        XCTAssertTrue(exported.contains { $0.payload["storageDropped"] == .integer(1) })
    }

    func testLightweightKeepsHealthAndSamplesPerFrameEvidence() async throws {
        let previous = UserDefaults.standard.bool(forKey: "ios.diagnostics.lightweight")
        let writer = IOSRuntimeTraceWriter(rootDirectory: try root(), profile: .dev)
        writer.setLightweight(true)
        defer { writer.setLightweight(previous) }
        for event in ["rtcHealthSnapshot", "displayHealthSnapshot", "runtimeSupervisionSnapshot"] {
            writer.record(draft(event, category: .snapshot))
        }
        for count in 1...120 {
            writer.record(
                draft("displayPresent", payload: ["presentCount": .integer(Int64(count))]))
        }
        let exported = try await rows(writer)
        XCTAssertEqual(exported.filter { $0.category == "snapshot" }.count, 3)
        XCTAssertEqual(exported.filter { $0.event == "displayPresent" }.count, 3)
        try await writer.startCapture()
        writer.record(draft("displayPresent", payload: ["presentCount": .integer(121)]))
        let captureURL = await writer.stopCapture()
        let url = try XCTUnwrap(captureURL)
        XCTAssertTrue(try String(contentsOf: url, encoding: .utf8).contains("displayPresent"))
    }

    func testPacketStatisticsUseVideoScopeAndPreserveMissingObservations() {
        var packets = StreamingVideoPacketStatistics()
        packets.observe(kind: "video", received: 100, lost: 2)
        packets.observe(kind: "audio", received: 500, lost: 80)
        packets.observe(kind: "video", received: 50, lost: 1)
        XCTAssertEqual(packets.received, 150)
        XCTAssertEqual(packets.lost, 3)
        XCTAssertEqual(packets.observedLoss, 3)
        packets.observe(kind: "video", received: 10, lost: nil)
        XCTAssertNil(packets.observedLoss)
    }

    func testCounterWindowsPreserveZeroSupplyAndResetOnGenerationOrRegression() {
        var window = StreamingTraceCounterWindow()
        XCTAssertEqual(
            window.observe(at: 1000, scope: "1", counters: ["presentCount": 2])[
                "presentCountDelta"],
            .null)
        let growing = window.observe(at: 2000, scope: "1", counters: ["presentCount": 62])
        XCTAssertEqual(growing["presentCountDelta"], .integer(60))
        XCTAssertEqual(growing["sampleWindowMs"], .double(1000))
        XCTAssertEqual(
            window.observe(at: 3000, scope: "1", counters: ["presentCount": 62])[
                "presentCountDelta"],
            .integer(0))
        let reset = window.observe(at: 4000, scope: "1", counters: ["presentCount": 1])
        XCTAssertEqual(reset["presentCountDelta"], .null)
        XCTAssertEqual(reset["counterReset"], .bool(true))
        XCTAssertEqual(
            window.observe(at: 5000, scope: "2", counters: ["presentCount": 60])[
                "presentCountDelta"],
            .null)
    }

    func testGpuAndDrawableLatencyHaveIndependentSourcesAndBoundedWindows() {
        let telemetry = StreamingDisplayTelemetry()
        for index in 1...300 {
            let submittedAt = Double(index) * 16
            _ = telemetry.recordSubmit(
                identifier: UInt64(index), timestampMilliseconds: submittedAt)
            _ = telemetry.recordPresent(
                identifier: UInt64(index), timestampMilliseconds: submittedAt + 2)
            _ = telemetry.recordDrawablePresent(
                submittedAt: submittedAt, presentedAt: submittedAt + 22)
        }
        let snapshot = telemetry.setDrawableBusy(false)
        XCTAssertEqual(snapshot.displayedFrameDelta, 1)
        XCTAssertEqual(snapshot.latencySampleCount, 240)
        XCTAssertEqual(snapshot.drawableSampleCount, 240)
        XCTAssertEqual(snapshot.submitToPresentP95Milliseconds, 2)
        XCTAssertEqual(snapshot.submitToDrawableP95Ms, 22)
        XCTAssertEqual(
            snapshot.tracePayload()["presentTimestampSource"], .string("commandBufferCompletion"))
        XCTAssertEqual(
            snapshot.tracePayload()["drawableTimestampSource"], .string("drawablePresentedTime"))
        XCTAssertEqual(snapshot.drawablePresentCount, 300)
    }
}
