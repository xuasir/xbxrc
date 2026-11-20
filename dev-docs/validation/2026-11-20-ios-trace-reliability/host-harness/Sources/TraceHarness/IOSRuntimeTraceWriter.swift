import Foundation
import OSLog

struct IOSRuntimeTraceStorageFailure: Error, LocalizedError, Sendable {
    let stage: String
    let code: Int
    var errorDescription: String? { "Trace 保存失败（\(stage)，错误 \(code)），请恢复存储后重新导出。" }
}

struct IOSRuntimeTraceWriterStatus: Sendable {
    let storageFailure: IOSRuntimeTraceStorageFailure?
    let storageFailures: Int
    let storageDroppedRows: Int
    let pendingRows: Int
    let highWaterRows: Int
    let bufferedBytes: Int
}

final class IOSRuntimeTraceWriter: @unchecked Sendable {
    private struct DropCounters {
        var essential = 0
        var key = 0
        var debug = 0
        var raw = 0
        var count: Int { essential + key + debug + raw }
        mutating func increment(_ importance: IOSRuntimeTraceImportance) {
            switch importance {
            case .essential: essential += 1
            case .key: key += 1
            case .debug: debug += 1
            case .raw: raw += 1
            }
        }
    }

    private let rootDirectory: URL
    private let queue: DispatchQueue
    private let queueKey = DispatchSpecificKey<Bool>()
    private let stateLock = NSLock()
    private let encoder: JSONEncoder
    private let logger: Logger
    private let launchSessionID: String
    private let fingerprintSalt: String
    private let budgetOverride: IOSRuntimeTraceBudget?

    private var storedProfile: IOSRuntimeTraceProfile
    private var effectiveProfileValue: IOSRuntimeTraceProfile
    private var pendingRows = 0
    private var highWaterRows = 0
    private var dropNoticeScheduled = false
    private var lastDropNoticeAtMs: Double = -.infinity
    private var totalDrops = DropCounters()
    private var sequence: UInt64 = 0
    private var fileID: UInt64 = 0
    private var fileHandle: FileHandle?
    private var activeFileURL: URL?
    private var bytesWritten: UInt64 = 0
    private var bufferedLines: [Data] = []
    private var bufferedBytes = 0
    private var storageFailure: IOSRuntimeTraceStorageFailure?
    private var storageFailures = 0
    private var storageDroppedRows = 0
    private var unreportedStorageDrops = 0
    private var durabilityUncertainRows = 0
    private var lastOpenAttemptAt: Double = -.infinity
    private var flushScheduled = false
    private var dropCounters = DropCounters()
    private var capture: IOSDiagnosticRecord?
    private var captureHandle: FileHandle?
    private var captureBytes: UInt64 = 0
    private var captureActive = false
    private var lastIssueAt = Date.distantPast
    private var lightweight = UserDefaults.standard.bool(forKey: "ios.diagnostics.lightweight")

    private var effectiveRecordingProfile: IOSRuntimeTraceProfile {
        stateLock.withLock {
            captureActive && effectiveProfileValue == .off ? .production : effectiveProfileValue
        }
    }

    init(
        rootDirectory: URL = IOSRuntimeTraceWriter.defaultRootDirectory(),
        profile: IOSRuntimeTraceProfile = IOSRuntimeTracePolicy.defaultProfile,
        launchSessionID: String = UUID().uuidString,
        budgetOverride: IOSRuntimeTraceBudget? = nil,
        queue: DispatchQueue? = nil
    ) {
        self.rootDirectory = rootDirectory
        storedProfile = profile
        effectiveProfileValue = IOSRuntimeTracePolicy.effectiveProfile(profile)
        self.launchSessionID = launchSessionID
        self.budgetOverride = budgetOverride
        fingerprintSalt = UUID().uuidString
        self.queue =
            queue ?? DispatchQueue(label: "com.xuasir.xbxrc.ios-runtime-trace", qos: .utility)
        self.queue.setSpecific(key: queueKey, value: true)
        encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        logger = Logger(
            subsystem: Bundle.main.bundleIdentifier ?? "com.xuasir.xbxrc.ios",
            category: "RuntimeTrace"
        )
        self.queue.async { [weak self] in
            self?.openFile(reason: "initial")
        }
    }

    deinit {
        // 异步任务可能持有最后一份引用；已在写入队列时直接清理，避免同步等待自身。
        if DispatchQueue.getSpecific(key: queueKey) == true {
            closeHandles()
        } else {
            queue.sync { closeHandles() }
        }
    }

    private func closeHandles() {
        emitDropNotice(force: true)
        flushBuffer()
        try? fileHandle?.close()
        try? captureHandle?.close()
    }

    var currentProfile: IOSRuntimeTraceProfile {
        stateLock.withLock { storedProfile }
    }

    var sessionID: String { launchSessionID }

    func status() async -> IOSRuntimeTraceWriterStatus {
        await withCheckedContinuation { continuation in
            queue.async {
                let pressure = self.stateLock.withLock { (self.pendingRows, self.highWaterRows) }
                continuation.resume(
                    returning: IOSRuntimeTraceWriterStatus(
                        storageFailure: self.storageFailure, storageFailures: self.storageFailures,
                        storageDroppedRows: self.storageDroppedRows, pendingRows: pressure.0,
                        highWaterRows: pressure.1, bufferedBytes: self.bufferedBytes))
            }
        }
    }

    func record(_ draft: IOSRuntimeTraceDraft) {
        let profile = effectiveRecordingProfile
        guard
            IOSRuntimeTracePolicy.shouldRecord(
                profile: profile,
                importance: draft.importance
            )
        else {
            return
        }

        let shouldSample = stateLock.withLock { lightweight && !captureActive }
        if shouldSample && !IOSRuntimeTracePolicy.shouldRecordLightweight(draft) {
            return
        }

        var shouldDrop = false
        var scheduleNotice = false
        stateLock.lock()
        if pendingRows >= IOSRuntimeTracePolicy.pendingLimit(for: draft.importance) {
            shouldDrop = true
            dropCounters.increment(draft.importance)
            totalDrops.increment(draft.importance)
            let now = ProcessInfo.processInfo.systemUptime * 1_000
            if !dropNoticeScheduled,
                now - lastDropNoticeAtMs >= Double(IOSRuntimeTracePolicy.budgetNoticeIntervalMs)
            {
                dropNoticeScheduled = true
                scheduleNotice = true
            }
        } else {
            pendingRows += 1
            highWaterRows = max(highWaterRows, pendingRows)
        }
        stateLock.unlock()

        if shouldDrop {
            if scheduleNotice {
                queue.async { [weak self] in self?.emitDropNotice(force: false) }
            }
            return
        }

        queue.async { [weak self] in
            guard let self else { return }
            self.append(draft)
            self.stateLock.withLock {
                self.pendingRows = max(0, self.pendingRows - 1)
            }
        }
    }

    func setProfile(_ profile: IOSRuntimeTraceProfile) {
        let effective = IOSRuntimeTracePolicy.effectiveProfile(profile)
        stateLock.withLock {
            storedProfile = profile
        }
        UserDefaults.standard.set(
            profile.rawValue, forKey: IOSRuntimeTracePolicy.profileDefaultsKey)
        queue.async { [weak self] in
            guard let self else { return }
            self.flushForBoundary()
            // 配置按IO队列顺序生效，已接收事件和尾部摘要先按原级别落盘。
            self.stateLock.withLock { self.effectiveProfileValue = effective }
            try? self.fileHandle?.close()
            self.fileHandle = nil
            self.activeFileURL = nil
            self.bytesWritten = 0
            self.openFile(reason: "traceConfigChanged", force: true)
        }
    }

    func flush() async {
        await withCheckedContinuation { continuation in
            queue.async { [weak self] in
                self?.flushForBoundary()
                continuation.resume()
            }
        }
    }

    func traceFiles() async -> [URL] {
        await withCheckedContinuation { continuation in
            queue.async { [weak self] in
                guard let self else {
                    continuation.resume(returning: [])
                    return
                }
                self.flushForBoundary()
                continuation.resume(returning: self.listTraceFiles())
            }
        }
    }

    func currentFileURL() async -> URL? {
        await withCheckedContinuation { continuation in
            queue.async { [weak self] in
                self?.flushForBoundary()
                continuation.resume(returning: self?.activeFileURL)
            }
        }
    }

    func clearFiles() async {
        await withCheckedContinuation { continuation in
            queue.async { [weak self] in
                guard let self else {
                    continuation.resume()
                    return
                }
                self.flushForBoundary()
                self.finishCaptureInternal()
                for record in self.captureRecords() {
                    try? FileManager.default.removeItem(at: self.captureURL(record))
                    try? FileManager.default.removeItem(at: self.captureMetadataURL(record))
                }
                try? self.fileHandle?.close()
                self.fileHandle = nil
                self.activeFileURL = nil
                for url in self.listTraceFiles() {
                    try? FileManager.default.removeItem(at: url)
                }
                self.bytesWritten = 0
                self.openFile(reason: "filesCleared", force: true)
                continuation.resume()
            }
        }
    }

    func prepareExport(allFiles: Bool) async throws -> URL {
        try await withCheckedThrowingContinuation { continuation in
            queue.async { [weak self] in
                guard let self else {
                    continuation.resume(throwing: CocoaError(.fileNoSuchFile))
                    return
                }
                do {
                    self.flushForBoundary()
                    if let failure = self.storageFailure { throw failure }
                    let files = self.listTraceFiles()
                    guard !files.isEmpty else {
                        throw CocoaError(.fileNoSuchFile)
                    }
                    if !allFiles {
                        if let activeFileURL = self.activeFileURL,
                            FileManager.default.fileExists(atPath: activeFileURL.path)
                        {
                            continuation.resume(returning: activeFileURL)
                            return
                        }
                        if let latestFileURL = files.last {
                            continuation.resume(returning: latestFileURL)
                            return
                        }
                    }
                    let exportURL = FileManager.default.temporaryDirectory
                        .appendingPathComponent("XBXRC-iOS-Trace-\(Self.nowMs()).jsonl")
                    FileManager.default.createFile(atPath: exportURL.path, contents: nil)
                    let output = try FileHandle(forWritingTo: exportURL)
                    defer { try? output.close() }
                    for file in files {
                        let input = try FileHandle(forReadingFrom: file)
                        defer { try? input.close() }
                        while let data = try input.read(upToCount: 256 * 1_024), !data.isEmpty {
                            try output.write(contentsOf: data)
                        }
                    }
                    continuation.resume(returning: exportURL)
                } catch {
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    func setLightweight(_ enabled: Bool) {
        stateLock.withLock { lightweight = enabled }
        UserDefaults.standard.set(enabled, forKey: "ios.diagnostics.lightweight")
    }

    func startCapture() async throws {
        try await withCheckedThrowingContinuation {
            (continuation: CheckedContinuation<Void, any Error>) in
            queue.async { [weak self] in
                guard let self else {
                    continuation.resume(throwing: CocoaError(.fileNoSuchFile))
                    return
                }
                guard self.capture == nil else {
                    continuation.resume(throwing: CocoaError(.fileWriteFileExists))
                    return
                }
                do {
                    try FileManager.default.createDirectory(
                        at: self.captureDirectory, withIntermediateDirectories: true)
                    self.flushForBoundary()
                    let record = IOSDiagnosticRecord(
                        id: UUID().uuidString, startedAt: Date(), endedAt: nil)
                    let url = self.captureURL(record)
                    guard FileManager.default.createFile(atPath: url.path, contents: nil) else {
                        throw CocoaError(.fileWriteUnknown)
                    }
                    self.captureHandle = try FileHandle(forWritingTo: url)
                    self.capture = record
                    self.captureBytes = 0
                    self.stateLock.withLock { self.captureActive = true }
                    try self.encoder.encode(record).write(
                        to: self.captureMetadataURL(record), options: .atomic)
                    self.rotateFile(
                        profile: self.effectiveRecordingProfile, reason: "diagnosticCaptureStarted")
                    self.pruneCaptureRecords()
                    self.queue.asyncAfter(deadline: .now() + 300) { [weak self] in
                        guard self?.capture?.id == record.id else { return }
                        self?.finishCaptureInternal()
                    }
                    continuation.resume()
                } catch {
                    self.finishCaptureInternal()
                    continuation.resume(throwing: error)
                }
            }
        }
    }

    func stopCapture() async -> URL? {
        await withCheckedContinuation { continuation in
            queue.async { [weak self] in
                guard let self, let record = self.capture else {
                    continuation.resume(returning: nil)
                    return
                }
                self.finishCaptureInternal()
                continuation.resume(returning: self.captureURL(record))
            }
        }
    }

    func diagnosticRecords() async -> [IOSDiagnosticRecord] {
        await withCheckedContinuation { continuation in
            queue.async { [weak self] in
                guard let self else {
                    continuation.resume(returning: [])
                    return
                }
                self.pruneCaptureRecords()
                continuation.resume(returning: self.captureRecords())
            }
        }
    }

    func diagnosticExport(_ record: IOSDiagnosticRecord) async throws -> URL {
        try await withCheckedThrowingContinuation { continuation in
            queue.async { [weak self] in
                guard let self else {
                    continuation.resume(throwing: CocoaError(.fileNoSuchFile))
                    return
                }
                do {
                    try self.captureHandle?.synchronize()
                    let source = self.captureURL(record)
                    let url = FileManager.default.temporaryDirectory.appendingPathComponent(
                        "XBXRC-Diagnostic-\(record.id).jsonl")
                    try? FileManager.default.removeItem(at: url)
                    try FileManager.default.copyItem(at: source, to: url)
                    continuation.resume(returning: url)
                } catch { continuation.resume(throwing: error) }
            }
        }
    }

    private var captureDirectory: URL {
        rootDirectory.appendingPathComponent("captures", isDirectory: true)
    }
    private func captureURL(_ record: IOSDiagnosticRecord) -> URL {
        captureDirectory.appendingPathComponent(record.id + ".jsonl")
    }
    private func captureMetadataURL(_ record: IOSDiagnosticRecord) -> URL {
        captureDirectory.appendingPathComponent(record.id + ".json")
    }

    private func saveIssueIfNeeded(_ draft: IOSRuntimeTraceDraft) {
        guard capture == nil, draft.importance <= .key,
            draft.event.lowercased().hasSuffix("failed"),
            ["auth", "ios-auth", "ios-streaming", "settings", "cloud-library"].contains(
                draft.domain),
            Date().timeIntervalSince(lastIssueAt) >= 60
        else { return }
        lastIssueAt = Date()
        guard let source = activeFileURL else { return }
        do {
            flushBuffer()
            try FileManager.default.createDirectory(
                at: captureDirectory, withIntermediateDirectories: true)
            let record = IOSDiagnosticRecord(
                id: UUID().uuidString, startedAt: Date(), endedAt: Date(),
                operation: draft.domain)
            try FileManager.default.copyItem(at: source, to: captureURL(record))
            try encoder.encode(record).write(to: captureMetadataURL(record), options: .atomic)
            pruneCaptureRecords()
        } catch { logger.error("Diagnostic snapshot could not be saved") }
    }

    private func appendCapture(_ data: Data) {
        guard let capture, let captureHandle else { return }
        guard captureBytes + UInt64(data.count) <= 32 * 1_024 * 1_024,
            Date().timeIntervalSince(capture.startedAt) < 300
        else {
            finishCaptureInternal()
            return
        }
        do {
            try captureHandle.write(contentsOf: data)
            captureBytes += UInt64(data.count)
        } catch { finishCaptureInternal() }
    }

    private func finishCaptureInternal() {
        guard var record = capture else { return }
        try? captureHandle?.synchronize()
        try? captureHandle?.close()
        captureHandle = nil
        record.endedAt = Date()
        try? encoder.encode(record).write(to: captureMetadataURL(record), options: .atomic)
        capture = nil
        stateLock.withLock { captureActive = false }
    }

    private func captureRecords() -> [IOSDiagnosticRecord] {
        let files =
            (try? FileManager.default.contentsOfDirectory(
                at: captureDirectory, includingPropertiesForKeys: nil)) ?? []
        return files.filter { $0.pathExtension == "json" }.compactMap {
            guard let data = try? Data(contentsOf: $0),
                var record = try? JSONDecoder().decode(IOSDiagnosticRecord.self, from: data),
                FileManager.default.fileExists(atPath: captureURL(record).path)
            else { return nil }
            record.isActive = record.id == capture?.id
            return record
        }.sorted { $0.startedAt > $1.startedAt }
    }

    private func pruneCaptureRecords() {
        for (index, record) in captureRecords().enumerated() where !record.isActive {
            if index >= 3 || Date().timeIntervalSince(record.startedAt) > 7 * 86_400 {
                try? FileManager.default.removeItem(at: captureURL(record))
                try? FileManager.default.removeItem(at: captureMetadataURL(record))
            }
        }
    }

    @discardableResult
    private func append(_ draft: IOSRuntimeTraceDraft) -> Bool {
        let profile = effectiveRecordingProfile
        guard IOSRuntimeTracePolicy.shouldRecord(profile: profile, importance: draft.importance)
        else { return false }
        if fileHandle == nil { openFile(reason: "writerRecovered") }
        var payload = IOSRuntimeTraceRedactor.redact(
            payload: draft.payload, profile: profile, fingerprintSalt: fingerprintSalt)
        payload["platform"] = .string("ios")
        payload["sourceMonotonicMs"] = .double(draft.monotonicMs)
        payload["writerQueueDelayMs"] = .double(
            max(0, ProcessInfo.processInfo.systemUptime * 1_000 - draft.monotonicMs))
        if let operationID = draft.operationID {
            payload["operationId"] = .string(
                IOSRuntimeTraceRedactor.sanitizeErrorMessage(operationID))
        }
        let nextSequence = sequence + 1
        guard
            var data = encode(
                draft: draft, payload: payload, profile: profile,
                sequence: nextSequence)
        else {
            countStorageDrops(1)
            return false
        }
        let budget = budget(for: profile)
        // 单行与编码缓冲都有独立上限；存储故障按丢失计数收口。
        guard data.count <= IOSRuntimeTracePolicy.maxRowBytes,
            UInt64(data.count) < budget.maxFileBytes
        else {
            countStorageDrops(1)
            return false
        }
        if fileHandle != nil, bytesWritten + UInt64(data.count) > budget.maxFileBytes {
            rotateFile(profile: profile, reason: "budgetRotate")
            guard
                let rotated = encode(
                    draft: draft, payload: payload, profile: profile,
                    sequence: sequence + 1)
            else {
                countStorageDrops(1)
                return false
            }
            data = rotated
        }
        sequence += 1
        appendCapture(data)
        guard fileHandle != nil,
            bytesWritten + UInt64(data.count) <= budget.maxFileBytes
        else {
            countStorageDrops(1)
            return false
        }
        if bufferedBytes + data.count > IOSRuntimeTracePolicy.maxBufferedBytes { flushBuffer() }
        guard fileHandle != nil else {
            countStorageDrops(1)
            return false
        }
        bufferedLines.append(data)
        bufferedBytes += data.count
        bytesWritten += UInt64(data.count)
        if draft.importance <= .key {
            logger.info("\(draft.domain, privacy: .public)/\(draft.event, privacy: .public)")
        }
        saveIssueIfNeeded(draft)
        if bufferedLines.count >= IOSRuntimeTracePolicy.batchRowLimit {
            flushBuffer()
        } else {
            scheduleFlushIfNeeded()
        }
        return fileHandle != nil
    }

    private func appendFileOpened(reason: String) {
        let profile = effectiveRecordingProfile
        guard profile != .off else { return }
        sequence += 1
        let budget = budget(for: profile)
        let envelope = IOSRuntimeTraceEnvelope(
            schemaVersion: IOSRuntimeTraceEnvelope.schemaVersion,
            seq: sequence,
            tsMs: Self.nowMs(),
            traceMode: profile.rawValue,
            traceProfile: profile.rawValue,
            dimension: IOSRuntimeTraceDimension.core.rawValue,
            importance: IOSRuntimeTraceImportance.essential.rawValue,
            category: IOSRuntimeTraceCategory.state.rawValue,
            domain: "trace",
            event: "fileOpened",
            sessionId: launchSessionID,
            payload: [
                "platform": .string("ios"),
                "reason": .string(reason),
                "maxFileBytes": .integer(Int64(budget.maxFileBytes)),
                "maxFiles": .integer(Int64(budget.maxFiles)),
                "sourceMonotonicMs": .double(ProcessInfo.processInfo.systemUptime * 1_000),
                "pendingRowLimit": .integer(Int64(IOSRuntimeTracePolicy.pendingRowLimit)),
                "maxRowBytes": .integer(Int64(IOSRuntimeTracePolicy.maxRowBytes)),
                "storageFailures": .integer(Int64(storageFailures)),
                "storageDroppedRows": .integer(Int64(storageDroppedRows)),
                "durabilityUncertainRows": .integer(Int64(durabilityUncertainRows)),
                "queueDroppedRowsTotal": .integer(Int64(stateLock.withLock { totalDrops.count })),
                "essentialDroppedTotal": .integer(
                    Int64(stateLock.withLock { totalDrops.essential })),
                "keyDroppedTotal": .integer(Int64(stateLock.withLock { totalDrops.key })),
                "debugDroppedTotal": .integer(Int64(stateLock.withLock { totalDrops.debug })),
                "rawDroppedTotal": .integer(Int64(stateLock.withLock { totalDrops.raw })),
            ]
        )
        guard var data = try? encoder.encode(envelope) else { return }
        data.append(0x0A)
        bufferedLines.append(data)
        bufferedBytes += data.count
        bytesWritten += UInt64(data.count)
        flushBuffer()
    }

    private func encode(
        draft: IOSRuntimeTraceDraft,
        payload: [String: IOSRuntimeTraceValue],
        profile: IOSRuntimeTraceProfile,
        sequence: UInt64
    ) -> Data? {
        let envelope = IOSRuntimeTraceEnvelope(
            schemaVersion: IOSRuntimeTraceEnvelope.schemaVersion,
            seq: sequence,
            tsMs: draft.tsMs,
            traceMode: profile.rawValue,
            traceProfile: profile.rawValue,
            dimension: draft.dimension.rawValue,
            importance: draft.importance.rawValue,
            category: draft.category.rawValue,
            domain: draft.domain,
            event: draft.event,
            sessionId: launchSessionID,
            payload: payload
        )
        guard var data = try? encoder.encode(envelope) else { return nil }
        data.append(0x0A)
        return data
    }

    private func openFile(reason: String, force: Bool = false) {
        let profile = effectiveRecordingProfile
        guard profile != .off else { return }
        let now = ProcessInfo.processInfo.systemUptime
        guard force || now - lastOpenAttemptAt >= IOSRuntimeTracePolicy.storageRetryInterval else {
            return
        }
        lastOpenAttemptAt = now
        let previousFailure = storageFailure
        do {
            try FileManager.default.createDirectory(
                at: rootDirectory, withIntermediateDirectories: true)
            fileID += 1
            let url = rootDirectory.appendingPathComponent(
                "runtime-trace-ios-\(Self.nowMs())-\(fileID).jsonl")
            guard FileManager.default.createFile(atPath: url.path, contents: nil) else {
                throw CocoaError(.fileWriteUnknown)
            }
            fileHandle = try FileHandle(forWritingTo: url)
            activeFileURL = url
            bytesWritten = 0
            storageFailure = nil
            pruneFiles(maxFiles: budget(for: profile).maxFiles)
            appendFileOpened(reason: reason)
            if previousFailure != nil, fileHandle != nil {
                append(
                    IOSRuntimeTraceDraft(
                        category: .state, domain: "trace",
                        event: "traceStorageRecovered",
                        payload: [
                            "failedStage": .string(previousFailure!.stage),
                            "errorCode": .integer(Int64(previousFailure!.code)),
                            "storageFailures": .integer(Int64(storageFailures)),
                            "storageDroppedRows": .integer(Int64(storageDroppedRows)),
                        ], dimension: .core, importance: .essential, operationID: nil))
            }
        } catch { failStorage(stage: "open", error: error, droppedRows: 0) }
    }

    private func rotateFile(profile: IOSRuntimeTraceProfile, reason: String) {
        flushBuffer()
        try? fileHandle?.close()
        fileHandle = nil
        activeFileURL = nil
        bytesWritten = 0
        openFile(reason: reason, force: true)
    }

    private func scheduleFlushIfNeeded() {
        guard !flushScheduled else { return }
        flushScheduled = true
        queue.asyncAfter(deadline: .now() + IOSRuntimeTracePolicy.flushInterval) { [weak self] in
            self?.flushBuffer()
        }
    }

    private func flushForBoundary() {
        if fileHandle == nil, storageFailure != nil {
            openFile(reason: "boundaryRetry", force: true)
        }
        emitDropNotice(force: true)
        flushBuffer()
    }

    private func flushBuffer() {
        flushScheduled = false
        guard let handle = fileHandle, !bufferedLines.isEmpty else { return }
        for (index, data) in bufferedLines.enumerated() {
            var startOffset: UInt64?
            do {
                startOffset = try handle.offset()
                try handle.write(contentsOf: data)
            } catch {
                // 回退可能的半行，保留完整JSONL前缀；未写尾部进入明确丢失计数。
                if let startOffset { try? handle.truncate(atOffset: startOffset) }
                failStorage(
                    stage: "write", error: error,
                    droppedRows: bufferedLines.count - index)
                return
            }
        }
        let synchronizedRows = bufferedLines.count
        bufferedLines.removeAll(keepingCapacity: true)
        bufferedBytes = 0
        do { try handle.synchronize() } catch {
            durabilityUncertainRows += synchronizedRows
            failStorage(stage: "synchronize", error: error, droppedRows: 0)
        }
    }

    private func countStorageDrops(_ count: Int) {
        storageDroppedRows += count
        unreportedStorageDrops += count
    }

    private func failStorage(stage: String, error: any Error, droppedRows: Int) {
        storageFailures += 1
        countStorageDrops(droppedRows)
        storageFailure = IOSRuntimeTraceStorageFailure(stage: stage, code: (error as NSError).code)
        logger.error(
            "Trace storage failed: \(stage, privacy: .public), code \((error as NSError).code)")
        try? fileHandle?.close()
        fileHandle = nil
        activeFileURL = nil
        bufferedLines.removeAll(keepingCapacity: false)
        bufferedBytes = 0
        bytesWritten = 0
    }

    private func emitDropNotice(force: Bool) {
        guard fileHandle != nil else {
            stateLock.withLock { dropNoticeScheduled = false }
            return
        }
        let now = ProcessInfo.processInfo.systemUptime * 1_000
        let counters = stateLock.withLock { () -> (DropCounters, DropCounters, Int)? in
            dropNoticeScheduled = false
            guard dropCounters.count > 0 || unreportedStorageDrops > 0 else { return nil }
            guard
                force
                    || now - lastDropNoticeAtMs
                        >= Double(IOSRuntimeTracePolicy.budgetNoticeIntervalMs)
            else { return nil }
            lastDropNoticeAtMs = now
            return (dropCounters, totalDrops, highWaterRows)
        }
        guard let (delta, totals, highWater) = counters else { return }
        let storageDelta = unreportedStorageDrops
        let recorded = append(
            IOSRuntimeTraceDraft(
                category: .state, domain: "trace",
                event: "traceBudgetNotice",
                payload: [
                    "reason": .string("writerPressureOrStorageLoss"), "finalSummary": .bool(force),
                    "essentialDropped": .integer(Int64(delta.essential)),
                    "keyDropped": .integer(Int64(delta.key)),
                    "debugDropped": .integer(Int64(delta.debug)),
                    "rawDropped": .integer(Int64(delta.raw)),
                    "essentialDroppedTotal": .integer(Int64(totals.essential)),
                    "keyDroppedTotal": .integer(Int64(totals.key)),
                    "debugDroppedTotal": .integer(Int64(totals.debug)),
                    "rawDroppedTotal": .integer(Int64(totals.raw)),
                    "storageDropped": .integer(Int64(storageDelta)),
                    "storageDroppedRows": .integer(Int64(storageDroppedRows)),
                    "pendingRows": .integer(Int64(stateLock.withLock { pendingRows })),
                    "highWaterRows": .integer(Int64(highWater)),
                    "pendingRowLimit": .integer(Int64(IOSRuntimeTracePolicy.pendingRowLimit)),
                ], dimension: .core, importance: .essential, operationID: nil))
        if recorded {
            stateLock.withLock {
                dropCounters.essential -= delta.essential
                dropCounters.key -= delta.key
                dropCounters.debug -= delta.debug
                dropCounters.raw -= delta.raw
            }
            unreportedStorageDrops = max(0, unreportedStorageDrops - storageDelta)
        }
    }

    private func listTraceFiles() -> [URL] {
        let urls =
            (try? FileManager.default.contentsOfDirectory(
                at: rootDirectory,
                includingPropertiesForKeys: [.contentModificationDateKey],
                options: [.skipsHiddenFiles]
            )) ?? []
        return
            urls
            .filter {
                $0.lastPathComponent.hasPrefix("runtime-trace-ios-")
                    && $0.pathExtension == "jsonl"
            }
            .sorted { lhs, rhs in
                let left = Self.traceFileOrder(lhs)
                let right = Self.traceFileOrder(rhs)
                if left.timestamp != right.timestamp {
                    return left.timestamp < right.timestamp
                }
                if left.fileID != right.fileID {
                    return left.fileID < right.fileID
                }
                return lhs.lastPathComponent < rhs.lastPathComponent
            }
    }

    private func pruneFiles(maxFiles: Int) {
        guard maxFiles > 0 else { return }
        let files = listTraceFiles()
        guard files.count > maxFiles else { return }
        for url in files.prefix(files.count - maxFiles) where url != activeFileURL {
            try? FileManager.default.removeItem(at: url)
        }
    }

    private func budget(for profile: IOSRuntimeTraceProfile) -> IOSRuntimeTraceBudget {
        budgetOverride ?? IOSRuntimeTracePolicy.budget(for: profile)
    }

    static func defaultRootDirectory() -> URL {
        let applicationSupport =
            FileManager.default.urls(
                for: .applicationSupportDirectory,
                in: .userDomainMask
            ).first ?? FileManager.default.temporaryDirectory
        return
            applicationSupport
            .appendingPathComponent("XBXRC", isDirectory: true)
            .appendingPathComponent("RuntimeTrace", isDirectory: true)
    }

    private static func nowMs() -> UInt64 {
        UInt64(max(0, Date().timeIntervalSince1970 * 1_000))
    }

    private static func traceFileOrder(_ url: URL) -> (timestamp: UInt64, fileID: UInt64) {
        let components = url.deletingPathExtension().lastPathComponent.split(separator: "-")
        guard components.count >= 2 else { return (0, 0) }
        return (
            UInt64(components[components.count - 2]) ?? 0,
            UInt64(components[components.count - 1]) ?? 0
        )
    }
}

extension NSLock {
    fileprivate func withLock<Value>(_ operation: () throws -> Value) rethrows -> Value {
        lock()
        defer { unlock() }
        return try operation()
    }
}
