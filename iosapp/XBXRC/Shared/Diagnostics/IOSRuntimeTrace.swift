import Foundation

enum IOSRuntimeTrace {
    private static let writer: IOSRuntimeTraceWriter = {
        let raw = UserDefaults.standard.string(forKey: IOSRuntimeTracePolicy.profileDefaultsKey)
        let stored = raw.flatMap(IOSRuntimeTraceProfile.init(rawValue:))
            ?? IOSRuntimeTracePolicy.defaultProfile
        return IOSRuntimeTraceWriter(profile: stored)
    }()

    static var currentProfile: IOSRuntimeTraceProfile {
        writer.currentProfile
    }

    static var launchSessionID: String {
        writer.sessionID
    }

    static func setProfile(_ profile: IOSRuntimeTraceProfile) {
        writer.setProfile(profile)
    }

    static func event(
        domain: String,
        event: String,
        payload: [String: IOSRuntimeTraceValue] = [:],
        dimension: IOSRuntimeTraceDimension = .core,
        importance: IOSRuntimeTraceImportance = .key,
        operationID: String? = nil
    ) {
        record(
            category: .event,
            domain: domain,
            event: event,
            payload: payload,
            dimension: dimension,
            importance: importance,
            operationID: operationID
        )
    }

    static func decision(
        domain: String,
        event: String,
        payload: [String: IOSRuntimeTraceValue] = [:],
        dimension: IOSRuntimeTraceDimension = .core,
        importance: IOSRuntimeTraceImportance = .key,
        operationID: String? = nil
    ) {
        record(
            category: .decision,
            domain: domain,
            event: event,
            payload: payload,
            dimension: dimension,
            importance: importance,
            operationID: operationID
        )
    }

    static func state(
        domain: String,
        event: String,
        payload: [String: IOSRuntimeTraceValue] = [:],
        dimension: IOSRuntimeTraceDimension = .core,
        importance: IOSRuntimeTraceImportance = .key,
        operationID: String? = nil
    ) {
        record(
            category: .state,
            domain: domain,
            event: event,
            payload: payload,
            dimension: dimension,
            importance: importance,
            operationID: operationID
        )
    }

    static func snapshot(
        domain: String,
        event: String,
        payload: [String: IOSRuntimeTraceValue] = [:],
        dimension: IOSRuntimeTraceDimension = .core,
        importance: IOSRuntimeTraceImportance = .key,
        operationID: String? = nil
    ) {
        record(
            category: .snapshot,
            domain: domain,
            event: event,
            payload: payload,
            dimension: dimension,
            importance: importance,
            operationID: operationID
        )
    }

    static func log(
        domain: String,
        event: String,
        payload: [String: IOSRuntimeTraceValue] = [:],
        dimension: IOSRuntimeTraceDimension = .engineLog,
        importance: IOSRuntimeTraceImportance = .debug,
        operationID: String? = nil
    ) {
        record(
            category: .log,
            domain: domain,
            event: event,
            payload: payload,
            dimension: dimension,
            importance: importance,
            operationID: operationID
        )
    }

    static func flush() async {
        await writer.flush()
    }

    static func traceFiles() async -> [URL] {
        await writer.traceFiles()
    }

    static func currentFileURL() async -> URL? {
        await writer.currentFileURL()
    }

    static func prepareExport(allFiles: Bool) async throws -> URL {
        try await writer.prepareExport(allFiles: allFiles)
    }

    static func clearFiles() async {
        await writer.clearFiles()
    }

    private static func record(
        category: IOSRuntimeTraceCategory,
        domain: String,
        event: String,
        payload: [String: IOSRuntimeTraceValue],
        dimension: IOSRuntimeTraceDimension,
        importance: IOSRuntimeTraceImportance,
        operationID: String?
    ) {
        writer.record(
            IOSRuntimeTraceDraft(
                category: category,
                domain: domain,
                event: event,
                payload: payload,
                dimension: dimension,
                importance: importance,
                operationID: operationID
            )
        )
    }
}
