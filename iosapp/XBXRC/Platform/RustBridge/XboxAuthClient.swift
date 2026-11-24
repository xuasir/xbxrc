import Foundation

typealias AuthCredentialCheckpoint = @MainActor @Sendable (XboxAuthCheckpoint) async throws -> Void
typealias StreamingAuthRenewal = @MainActor @Sendable (String, String?) async throws -> AuthSession

protocol XboxAuthClient: Sendable {
    func beginLogin() async throws -> LoginStartResult
    func finishLogin(
        callbackURL: URL,
        pendingJSON: String,
        seedJSON: String,
        forceRegionIP: String
    ) async throws -> AuthSession
    func renewLogin(
        session: StoredAuthSession,
        forceRegionIP: String,
        scope: XboxAuthScope,
        forceCoreRefresh: Bool,
        rejectedToken: String?,
        checkpoint: @escaping AuthCredentialCheckpoint
    ) async throws -> AuthSession
    func loadProfileSource(webTokenJSON: String, source: XboxProfileSource) async throws
        -> XboxProfile
    func loadProfile(webTokenJSON: String) async throws -> XboxProfile
}

struct RustXboxAuthClient: XboxAuthClient {
    func beginLogin() async throws -> LoginStartResult {
        try await traceBoundary(event: "beginLogin") {
            try await startLogin()
        }
    }

    func finishLogin(
        callbackURL: URL,
        pendingJSON: String,
        seedJSON: String,
        forceRegionIP: String
    ) async throws -> AuthSession {
        try await traceBoundary(event: "finishLogin") {
            try await completeLogin(
                callbackUrl: callbackURL.absoluteString,
                pendingJson: pendingJSON,
                seedJson: seedJSON,
                forceRegionIp: forceRegionIP
            )
        }
    }

    func renewLogin(
        session: StoredAuthSession,
        forceRegionIP: String,
        scope: XboxAuthScope,
        forceCoreRefresh: Bool,
        rejectedToken: String?,
        checkpoint: @escaping AuthCredentialCheckpoint
    ) async throws -> AuthSession {
        try await traceBoundary(event: "renewLogin") {
            try await refreshLogin(
                session: session.bridgeSession,
                forceRegionIp: forceRegionIP,
                scope: scope,
                forceCoreRefresh: forceCoreRefresh,
                rejectedToken: rejectedToken,
                observer: RustAuthCheckpointObserver(checkpoint: checkpoint)
            )
        }
    }

    func loadProfile(webTokenJSON: String) async throws -> XboxProfile {
        try await traceBoundary(event: "loadProfile") {
            try await fetchProfile(webTokenJson: webTokenJSON)
        }
    }

    func loadProfileSource(webTokenJSON: String, source: XboxProfileSource) async throws
        -> XboxProfile
    {
        try await traceBoundary(event: "loadProfileSource") {
            try await fetchProfileSection(webTokenJson: webTokenJSON, source: source)
        }
    }

    private func traceBoundary<Value: Sendable>(
        event: String,
        operation: () async throws -> Value
    ) async throws -> Value {
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "auth",
            event: "uniffiBoundaryStarted",
            payload: ["operation": .string(event)],
            dimension: .network,
            importance: .debug,
            operationID: operationID
        )
        do {
            let result = try await operation()
            IOSRuntimeTrace.event(
                domain: "auth",
                event: "uniffiBoundarySucceeded",
                payload: [
                    "operation": .string(event),
                    "elapsedMs": .integer(
                        Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .network,
                importance: .debug,
                operationID: operationID
            )
            return result
        } catch {
            IOSRuntimeTrace.event(
                domain: "auth",
                event: "uniffiBoundaryFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "operation": .string(event),
                        "elapsedMs": .integer(
                            Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ]
                ),
                dimension: .network,
                importance: .essential,
                operationID: operationID
            )
            throw error
        }
    }
}

final class RustAuthCheckpointObserver: XboxAuthObserver, Sendable {
    private let persist: AuthCredentialCheckpoint
    private let operationID: String

    init(checkpoint: @escaping AuthCredentialCheckpoint, operationID: String = UUID().uuidString) {
        persist = checkpoint
        self.operationID = operationID
    }

    func checkpoint(update: XboxAuthCheckpoint) async throws {
        do { try await persist(update) } catch is CancellationError {
            throw XboxBridgeError.Cancelled
        } catch let error as XboxBridgeError { throw error } catch {
            IOSRuntimeTrace.event(
                domain: "auth", event: "credentialCheckpointFailed",
                payload: ["coreOnly": .bool(update.coreOnly)], dimension: .lifecycle,
                importance: .essential, operationID: operationID)
            if AuthStoreError.isStale(error) { throw XboxBridgeError.Cancelled }
            throw XboxBridgeError.InvalidData("credentialCheckpointFailed")
        }
    }

    func onStage(event: XboxAuthStage) {
        IOSRuntimeTrace.event(
            domain: "auth", event: "credentialFlowStage", payload: Self.stagePayload(event),
            dimension: .network, importance: event.failure == nil ? .key : .essential,
            operationID: operationID)
    }

    static func stagePayload(_ event: XboxAuthStage) -> [String: IOSRuntimeTraceValue] {
        var payload: [String: IOSRuntimeTraceValue] = ["stage": .string(event.stage)]
        if let stage = event.requestStage { payload["requestStage"] = .string(stage) }
        if let scope = event.scope { payload["scope"] = .string(scope) }
        if let elapsed = event.elapsedMs {
            payload["elapsedMs"] = .integer(Int64(clamping: elapsed))
        }
        if let skipCore = event.skipCoreRefresh { payload["skipCoreRefresh"] = .bool(skipCore) }
        if let kind = event.networkErrorKind { payload["networkErrorKind"] = .string(kind) }
        if let failure = event.failure {
            payload["errorKind"] = .string(String(describing: failure.kind))
            payload["retriable"] = .bool(failure.retriable)
            if let status = failure.statusCode { payload["statusCode"] = .integer(Int64(status)) }
        }
        return payload
    }
}

final class RustStreamingAuthProvider: XboxStreamingAuthProvider, Sendable {
    private let renew: StreamingAuthRenewal
    init(renew: @escaping StreamingAuthRenewal) { self.renew = renew }
    func ensureSession(targetType: String, rejectedToken: String?) async throws -> AuthSession {
        do { return try await renew(targetType, rejectedToken) } catch is CancellationError {
            throw XboxBridgeError.Cancelled
        } catch let error as XboxBridgeError { throw error } catch {
            if AuthStoreError.isStale(error) { throw XboxBridgeError.Cancelled }
            throw XboxBridgeError.InvalidData("streamingCredentialOwnerUnavailable")
        }
    }
}

extension XboxAuthClient {
    func loadProfileSource(webTokenJSON: String, source: XboxProfileSource) async throws
        -> XboxProfile
    {
        try await loadProfile(webTokenJSON: webTokenJSON)
    }
}
