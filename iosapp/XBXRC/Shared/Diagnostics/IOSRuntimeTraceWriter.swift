import Foundation
import OSLog

final class IOSRuntimeTraceWriter: @unchecked Sendable {
    private struct DropCounters {
        var debug = 0
        var raw = 0
        var lastNoticeAtMs: UInt64 = 0
    }

    private let rootDirectory: URL
    private let queue: DispatchQueue
    private let stateLock = NSLock()
    private let encoder: JSONEncoder
    private let logger: Logger
    private let launchSessionID: String
    private let fingerprintSalt: String
    private let budgetOverride: IOSRuntimeTraceBudget?

    private var storedProfile: IOSRuntimeTraceProfile
    private var effectiveProfileValue: IOSRuntimeTraceProfile
    private var pendingRows = 0
    private var sequence: UInt64 = 0
    private var fileID: UInt64 = 0
    private var fileHandle: FileHandle?
    private var activeFileURL: URL?
    private var bytesWritten: UInt64 = 0
    private var bufferedLines: [Data] = []
    private var flushScheduled = false
    private var dropCounters = DropCounters()

    init(
        rootDirectory: URL = IOSRuntimeTraceWriter.defaultRootDirectory(),
        profile: IOSRuntimeTraceProfile = IOSRuntimeTracePolicy.defaultProfile,
        launchSessionID: String = UUID().uuidString,
        budgetOverride: IOSRuntimeTraceBudget? = nil
    ) {
        self.rootDirectory = rootDirectory
        storedProfile = profile
        effectiveProfileValue = IOSRuntimeTracePolicy.effectiveProfile(profile)
        self.launchSessionID = launchSessionID
        self.budgetOverride = budgetOverride
        fingerprintSalt = UUID().uuidString
        queue = DispatchQueue(label: "com.xuasir.xbxrc.ios-runtime-trace", qos: .utility)
        encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        logger = Logger(
            subsystem: Bundle.main.bundleIdentifier ?? "com.xuasir.xbxrc.ios",
            category: "RuntimeTrace"
        )
        queue.async { [weak self] in
            self?.openFile(reason: "initial")
        }
    }

    deinit {
        queue.sync {
            flushBuffer()
            try? fileHandle?.close()
        }
    }

    var currentProfile: IOSRuntimeTraceProfile {
        stateLock.withLock { storedProfile }
    }

    var sessionID: String { launchSessionID }

    func record(_ draft: IOSRuntimeTraceDraft) {
        let profile = stateLock.withLock { effectiveProfileValue }
        guard IOSRuntimeTracePolicy.shouldRecord(
            profile: profile,
            importance: draft.importance
        ) else {
            return
        }

        var shouldDrop = false
        stateLock.lock()
        if pendingRows >= IOSRuntimeTracePolicy.pendingRowLimit,
           draft.importance > .key {
            shouldDrop = true
        } else {
            pendingRows += 1
        }
        stateLock.unlock()

        if shouldDrop {
            queue.async { [weak self] in
                self?.recordDrop(importance: draft.importance)
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
            effectiveProfileValue = effective
        }
        UserDefaults.standard.set(profile.rawValue, forKey: IOSRuntimeTracePolicy.profileDefaultsKey)
        queue.async { [weak self] in
            guard let self else { return }
            self.flushBuffer()
            try? self.fileHandle?.close()
            self.fileHandle = nil
            self.activeFileURL = nil
            self.bytesWritten = 0
            self.openFile(reason: "traceConfigChanged")
        }
    }

    func flush() async {
        await withCheckedContinuation { continuation in
            queue.async { [weak self] in
                self?.flushBuffer()
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
                self.flushBuffer()
                continuation.resume(returning: self.listTraceFiles())
            }
        }
    }

    func currentFileURL() async -> URL? {
        await withCheckedContinuation { continuation in
            queue.async { [weak self] in
                self?.flushBuffer()
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
                self.flushBuffer()
                try? self.fileHandle?.close()
                self.fileHandle = nil
                self.activeFileURL = nil
                for url in self.listTraceFiles() {
                    try? FileManager.default.removeItem(at: url)
                }
                self.bytesWritten = 0
                self.openFile(reason: "filesCleared")
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
                    self.flushBuffer()
                    let files = self.listTraceFiles()
                    guard !files.isEmpty else {
                        throw CocoaError(.fileNoSuchFile)
                    }
                    if !allFiles {
                        if let activeFileURL = self.activeFileURL,
                           FileManager.default.fileExists(atPath: activeFileURL.path) {
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

    private func append(_ draft: IOSRuntimeTraceDraft) {
        let profile = stateLock.withLock { effectiveProfileValue }
        guard profile != .off else { return }
        if fileHandle == nil {
            openFile(reason: "writerRecovered")
        }
        var payload = IOSRuntimeTraceRedactor.redact(
            payload: draft.payload,
            profile: profile,
            fingerprintSalt: fingerprintSalt
        )
        payload["platform"] = .string("ios")
        if let operationID = draft.operationID {
            payload["operationId"] = .string(
                IOSRuntimeTraceRedactor.sanitizeErrorMessage(operationID)
            )
        }

        var nextSequence = sequence + 1
        guard var data = encode(
            draft: draft,
            payload: payload,
            profile: profile,
            sequence: nextSequence
        ) else { return }
        let budget = budget(for: profile)
        if budget.maxFileBytes > 0,
           bytesWritten > 0,
           bytesWritten + UInt64(data.count) > budget.maxFileBytes {
            rotateFile(profile: profile, reason: "budgetRotate")
            nextSequence = sequence + 1
            guard let rotatedData = encode(
                draft: draft,
                payload: payload,
                profile: profile,
                sequence: nextSequence
            ) else { return }
            data = rotatedData
        }
        sequence = nextSequence
        bufferedLines.append(data)
        bytesWritten += UInt64(data.count)
        if draft.importance <= .key {
            logger.info("\(draft.domain, privacy: .public)/\(draft.event, privacy: .public)")
        }
        if bufferedLines.count >= IOSRuntimeTracePolicy.batchRowLimit {
            flushBuffer()
        } else {
            scheduleFlushIfNeeded()
        }
    }

    private func appendFileOpened(reason: String) {
        let profile = stateLock.withLock { effectiveProfileValue }
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
            ]
        )
        guard var data = try? encoder.encode(envelope) else { return }
        data.append(0x0A)
        bufferedLines.append(data)
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
            tsMs: Self.nowMs(),
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

    private func openFile(reason: String) {
        let profile = stateLock.withLock { effectiveProfileValue }
        guard profile != .off else { return }
        try? FileManager.default.createDirectory(
            at: rootDirectory,
            withIntermediateDirectories: true
        )
        fileID += 1
        let url = rootDirectory.appendingPathComponent(
            "runtime-trace-ios-\(Self.nowMs())-\(fileID).jsonl"
        )
        FileManager.default.createFile(atPath: url.path, contents: nil)
        fileHandle = try? FileHandle(forWritingTo: url)
        activeFileURL = url
        bytesWritten = 0
        pruneFiles(maxFiles: budget(for: profile).maxFiles)
        appendFileOpened(reason: reason)
    }

    private func rotateFile(profile: IOSRuntimeTraceProfile, reason: String) {
        flushBuffer()
        try? fileHandle?.close()
        fileHandle = nil
        activeFileURL = nil
        bytesWritten = 0
        openFile(reason: reason)
        pruneFiles(maxFiles: budget(for: profile).maxFiles)
    }

    private func scheduleFlushIfNeeded() {
        guard !flushScheduled else { return }
        flushScheduled = true
        queue.asyncAfter(deadline: .now() + IOSRuntimeTracePolicy.flushInterval) { [weak self] in
            self?.flushBuffer()
        }
    }

    private func flushBuffer() {
        flushScheduled = false
        guard let fileHandle, !bufferedLines.isEmpty else { return }
        for data in bufferedLines {
            try? fileHandle.write(contentsOf: data)
        }
        bufferedLines.removeAll(keepingCapacity: true)
        try? fileHandle.synchronize()
    }

    private func recordDrop(importance: IOSRuntimeTraceImportance) {
        switch importance {
        case .raw: dropCounters.raw += 1
        case .debug: dropCounters.debug += 1
        default: return
        }
        let now = Self.nowMs()
        guard dropCounters.lastNoticeAtMs == 0
            || now - dropCounters.lastNoticeAtMs >= IOSRuntimeTracePolicy.budgetNoticeIntervalMs else {
            return
        }
        let payload: [String: IOSRuntimeTraceValue] = [
            "reason": .string("writerQueuePressure"),
            "debugDropped": .integer(Int64(dropCounters.debug)),
            "rawDropped": .integer(Int64(dropCounters.raw)),
            "pendingRows": .integer(Int64(stateLock.withLock { pendingRows })),
        ]
        dropCounters = DropCounters(lastNoticeAtMs: now)
        append(
            IOSRuntimeTraceDraft(
                category: .state,
                domain: "trace",
                event: "traceBudgetNotice",
                payload: payload,
                dimension: .core,
                importance: .essential,
                operationID: nil
            )
        )
    }

    private func listTraceFiles() -> [URL] {
        let urls = (try? FileManager.default.contentsOfDirectory(
            at: rootDirectory,
            includingPropertiesForKeys: [.contentModificationDateKey],
            options: [.skipsHiddenFiles]
        )) ?? []
        return urls
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
        let applicationSupport = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? FileManager.default.temporaryDirectory
        return applicationSupport
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

private extension NSLock {
    func withLock<Value>(_ operation: () throws -> Value) rethrows -> Value {
        lock()
        defer { unlock() }
        return try operation()
    }
}
