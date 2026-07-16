import Foundation

protocol XboxAuthClient: Sendable {
    func beginLogin() async throws -> LoginStartResult
    func finishLogin(
        callbackURL: URL,
        pendingJSON: String,
        seedJSON: String,
        forceRegionIP: String
    ) async throws -> AuthSession
    func renewLogin(
        refreshToken: String,
        seedJSON: String,
        forceRegionIP: String
    ) async throws -> AuthSession
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
        refreshToken: String,
        seedJSON: String,
        forceRegionIP: String
    ) async throws -> AuthSession {
        try await traceBoundary(event: "renewLogin") {
            try await refreshLogin(
                refreshToken: refreshToken,
                seedJson: seedJSON,
                forceRegionIp: forceRegionIP
            )
        }
    }

    func loadProfile(webTokenJSON: String) async throws -> XboxProfile {
        try await traceBoundary(event: "loadProfile") {
            try await fetchProfile(webTokenJson: webTokenJSON)
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
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
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
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
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
