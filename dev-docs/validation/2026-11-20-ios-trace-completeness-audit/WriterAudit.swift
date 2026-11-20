import Foundation

@main struct WriterAudit {
    static let root = URL(fileURLWithPath: "/private/tmp/xbxrc-ios-trace-audit-20261120/r1/fixtures", isDirectory: true)
    static func child<T>(_ writer: IOSRuntimeTraceWriter, _ name: String, as type: T.Type) -> T? {
        Mirror(reflecting: writer).children.first { $0.label == name }?.value as? T
    }
    static func draft(_ event: String, category: IOSRuntimeTraceCategory = .state, importance: IOSRuntimeTraceImportance = .key, payload: [String: IOSRuntimeTraceValue] = [:]) -> IOSRuntimeTraceDraft {
        IOSRuntimeTraceDraft(category: category, domain: "ios-streaming", event: event, payload: payload, dimension: .core, importance: importance, operationID: "audit-attempt")
    }
    static func rows(_ writer: IOSRuntimeTraceWriter) async throws -> [IOSRuntimeTraceEnvelope] {
        var all: [IOSRuntimeTraceEnvelope] = []
        for file in await writer.traceFiles() {
            let text = try String(contentsOf: file, encoding: .utf8)
            for line in text.split(separator: "\n") {
                all.append(try JSONDecoder().decode(IOSRuntimeTraceEnvelope.self, from: Data(line.utf8)))
            }
        }
        return all
    }
    static func main() async throws {
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        UserDefaults.standard.setVolatileDomain(["ios.diagnostics.lightweight": false], forName: UserDefaults.argumentDomain)
        var results: [String: Any] = [:]

        let blocked = root.appendingPathComponent("blocked-root")
        try Data("file".utf8).write(to: blocked)
        let bad = IOSRuntimeTraceWriter(rootDirectory: blocked, profile: .dev)
        for _ in 0..<5 { bad.record(draft("auditEvent")) }
        await bad.flush()
        let before = child(bad, "bufferedLines", as: [Data].self)?.count ?? -1
        for _ in 0..<5 { bad.record(draft("auditEvent")) }
        await bad.flush()
        let after = child(bad, "bufferedLines", as: [Data].self)?.count ?? -1
        let reported = await bad.currentFileURL()
        results["openFailure"] = ["first5BufferedRows": before, "after10BufferedRows": after,
            "reportedActiveFile": reported != nil, "activeFileExists": reported.map { FileManager.default.fileExists(atPath: $0.path) } ?? false,
            "actualFiles": await bad.traceFiles().count]

        let closed = IOSRuntimeTraceWriter(rootDirectory: root.appendingPathComponent("closed-handle"), profile: .dev)
        await closed.flush()
        let handle = child(closed, "fileHandle", as: Optional<FileHandle>.self)!
        try handle?.close()
        closed.record(draft("eventAfterHandleClosed"))
        await closed.flush()
        results["writeFailure"] = ["eventPersisted": try await rows(closed).contains { $0.event == "eventAfterHandleClosed" },
            "bufferedRowsAfterFailedWrite": child(closed, "bufferedLines", as: [Data].self)?.count ?? -1,
            "sequenceAfterFailedWrite": child(closed, "sequence", as: UInt64.self) ?? 0]

        UserDefaults.standard.setVolatileDomain(["ios.diagnostics.lightweight": true], forName: UserDefaults.argumentDomain)
        let light = IOSRuntimeTraceWriter(rootDirectory: root.appendingPathComponent("lightweight"), profile: .dev)
        for name in ["rtcHealthSnapshot", "displayHealthSnapshot", "runtimeSupervisionSnapshot"] {
            light.record(draft(name, category: .snapshot))
        }
        light.record(draft("terminalSelected", importance: .essential))
        await light.flush()
        results["lightweight"] = ["retainedEvents": try await rows(light).map(\.event)]
        try await light.startCapture()
        light.record(draft("rtcHealthSnapshot", category: .snapshot))
        let capture = await light.stopCapture()!
        let captureRows = try String(contentsOf: capture, encoding: .utf8).split(separator: "\n").map {
            try JSONDecoder().decode(IOSRuntimeTraceEnvelope.self, from: Data($0.utf8))
        }
        results["manualCapture"] = ["retainedEvents": captureRows.map(\.event), "fileOpenedPresent": captureRows.contains { $0.event == "fileOpened" }]
        UserDefaults.standard.setVolatileDomain(["ios.diagnostics.lightweight": false], forName: UserDefaults.argumentDomain)

        let q = DispatchQueue(label: "audit.pressure")
        q.suspend()
        let pressure = IOSRuntimeTraceWriter(rootDirectory: root.appendingPathComponent("pressure"), profile: .dev, queue: q)
        for _ in 0..<5000 { pressure.record(draft("keyEvent")) }
        for _ in 0..<50 { pressure.record(draft("debugEvent", importance: .debug)) }
        results["pressureBeforeResume"] = ["pendingKeyRows": child(pressure, "pendingRows", as: Int.self) ?? -1, "configuredLimit": IOSRuntimeTracePolicy.pendingRowLimit]
        q.resume()
        await pressure.flush()
        let pressureRows = try await rows(pressure)
        let notices = pressureRows.filter { $0.event == "traceBudgetNotice" }
        results["pressureAfterFlush"] = ["keyEvents": pressureRows.filter { $0.event == "keyEvent" }.count, "debugEvents": pressureRows.filter { $0.event == "debugEvent" }.count,
            "noticeRows": notices.count, "reportedDebugDropped": notices.reduce(0) { sum, row in if case .integer(let n) = row.payload["debugDropped"] { sum + Int(n) } else { sum } },
            "unreportedDropCounters": child(pressure, "dropCounters", as: Any.self).map { String(describing: $0) } ?? "missing"]

        let tq = DispatchQueue(label: "audit.timestamps")
        tq.suspend()
        let timed = IOSRuntimeTraceWriter(rootDirectory: root.appendingPathComponent("timestamp"), profile: .dev, queue: tq)
        let sourceMs = Date().timeIntervalSince1970 * 1000
        timed.record(draft("delayedEvent"))
        try await Task.sleep(for: .milliseconds(180))
        tq.resume()
        await timed.flush()
        let delayed = try await rows(timed).first { $0.event == "delayedEvent" }!
        results["timestamp"] = ["sourceToTraceDelayMs": Double(delayed.tsMs) - sourceMs]

        let terminal = IOSRuntimeTraceWriter(rootDirectory: root.appendingPathComponent("terminal"), profile: .dev)
        terminal.record(draft("sessionTerminal", payload: ["reason": .string("runtime.failed")]))
        terminal.record(draft("terminalSelected", importance: .essential, payload: ["reason": .string("runtime.failed")]))
        await terminal.flush()
        let issueBefore = await terminal.diagnosticRecords().count
        terminal.record(draft("auditFailed"))
        await terminal.flush()
        results["automaticIssue"] = ["recordsAfterRuntimeFailureTerminal": issueBefore, "recordsAfterFailedSuffix": await terminal.diagnosticRecords().count]

        let out = try JSONSerialization.data(withJSONObject: results, options: [.prettyPrinted, .sortedKeys])
        try out.write(to: root.deletingLastPathComponent().appendingPathComponent("writer-probes.json"))
        print(String(decoding: out, as: UTF8.self))
    }
}
