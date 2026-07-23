import AuthenticationServices
import Combine
import Foundation

enum AuthPhase: Equatable {
    case restoring
    case signedOut
    case authenticating
    case refreshing
    case signedIn
    case failed
}

enum AuthProfileRefreshReason: String, Equatable, Sendable {
    case initialActivation
    case manualPull
    case manualRetry
}

@MainActor
final class AuthStore: ObservableObject {
    @Published private(set) var phase: AuthPhase = .restoring
    @Published private(set) var profile: XboxProfile?
    @Published private(set) var session: StoredAuthSession?
    @Published private(set) var ownerGeneration: UInt64 = 0
    @Published var errorMessage: String?

    private let client: any XboxAuthClient
    private let cloudClient: any XboxCloudDataClient
    private let keychain: any AuthSessionStoring
    private let webAuthentication: any WebAuthenticating
    private let settings: any CloudRegionSettingsProviding
    private var restored = false
    private var profileInitialRefreshGeneration: UInt64?
    private var profileRefreshTask: Task<Void, Never>?
    private var profileRefreshTaskID: UUID?

    init(
        client: any XboxAuthClient = RustXboxAuthClient(),
        cloudClient: any XboxCloudDataClient = RustXboxCloudDataClient(),
        keychain: any AuthSessionStoring = KeychainSessionStore(),
        webAuthentication: any WebAuthenticating = WebAuthenticationPresenter()
    ) {
        self.client = client
        self.cloudClient = cloudClient
        self.keychain = keychain
        self.webAuthentication = webAuthentication
        settings = AppSettingsStore()
    }

    init(
        settings: any CloudRegionSettingsProviding,
        client: any XboxAuthClient = RustXboxAuthClient(),
        cloudClient: any XboxCloudDataClient = RustXboxCloudDataClient(),
        keychain: any AuthSessionStoring = KeychainSessionStore(),
        webAuthentication: any WebAuthenticating = WebAuthenticationPresenter()
    ) {
        self.client = client
        self.cloudClient = cloudClient
        self.keychain = keychain
        self.webAuthentication = webAuthentication
        self.settings = settings
    }

    var isSignedIn: Bool {
        session != nil
    }

    var isBusy: Bool {
        phase == .restoring || phase == .authenticating || phase == .refreshing
    }

    func restore() async {
        guard !restored else {
            IOSRuntimeTrace.decision(
                domain: "auth",
                event: "authRestoreSkipped",
                payload: ["reason": "alreadyRestored"],
                dimension: .lifecycle,
                importance: .debug
            )
            return
        }
        let operationID = UUID().uuidString
        let startedAt = Date()
        restored = true
        phase = .restoring
        IOSRuntimeTrace.event(
            domain: "auth",
            event: "authRestoreStarted",
            payload: [:],
            dimension: .lifecycle,
            importance: .key,
            operationID: operationID
        )

        do {
            guard let stored = try await keychain.load() else {
                phase = .signedOut
                IOSRuntimeTrace.state(
                    domain: "auth",
                    event: "authRestoreSucceeded",
                    payload: [
                        "result": "signedOut",
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ],
                    dimension: .lifecycle,
                    importance: .key,
                    operationID: operationID
                )
                return
            }
            profile = nil
            IOSRuntimeTrace.decision(
                domain: "auth",
                event: "authRestoreSessionFound",
                payload: ["action": "renewCredentials"],
                dimension: .lifecycle,
                importance: .key,
                operationID: operationID
            )
            await refreshSession(stored, markSignedIn: false)
            ownerGeneration &+= 1
            profileInitialRefreshGeneration = nil
            phase = .signedIn
            IOSRuntimeTrace.state(
                domain: "auth",
                event: "authRestoreSucceeded",
                payload: [
                    "result": .string(String(describing: phase)),
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .lifecycle,
                importance: .key,
                operationID: operationID
            )
        } catch {
            phase = .failed
            errorMessage = error.localizedDescription
            IOSRuntimeTrace.event(
                domain: "auth",
                event: "authRestoreFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ]
                ),
                dimension: .lifecycle,
                importance: .essential,
                operationID: operationID
            )
        }
    }

    func signIn() async {
        guard !isBusy else {
            IOSRuntimeTrace.decision(
                domain: "auth",
                event: "authSignInSkipped",
                payload: ["phase": .string(String(describing: phase))],
                dimension: .lifecycle,
                importance: .debug
            )
            return
        }
        let operationID = UUID().uuidString
        let startedAt = Date()
        phase = .authenticating
        errorMessage = nil
        IOSRuntimeTrace.event(
            domain: "auth",
            event: "authSignInStarted",
            payload: [:],
            dimension: .lifecycle,
            importance: .key,
            operationID: operationID
        )

        do {
            let start = try await client.beginLogin()
            let callbackURL = try await webAuthentication.authenticate(
                authorizationURL: start.authorizationUrl,
                prefersEphemeralSession: settings.usesEphemeralLoginSession
            )
            let bridgeSession = try await client.finishLogin(
                callbackURL: callbackURL,
                pendingJSON: start.pendingJson,
                seedJSON: start.seedJson,
                forceRegionIP: settings.cloudRegionPreset.forceRegionIP
            )
            let stored = StoredAuthSession(bridgeSession: bridgeSession)
            try await keychain.save(stored)
            session = stored
            ownerGeneration &+= 1
            profileInitialRefreshGeneration = nil
            profile = nil
            phase = .signedIn
            IOSRuntimeTrace.state(
                domain: "auth",
                event: "authSignInSucceeded",
                payload: [
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    "profileLoaded": false,
                    "appLevel": .integer(Int64(stored.appLevel)),
                    "regionPreset": .string(settings.cloudRegionPreset.rawValue),
                ],
                dimension: .lifecycle,
                importance: .key,
                operationID: operationID
            )
        } catch let error as ASWebAuthenticationSessionError
            where error.code == .canceledLogin {
            phase = .signedOut
            IOSRuntimeTrace.decision(
                domain: "auth",
                event: "authSignInCancelled",
                payload: [
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    "reason": "userCancelled",
                ],
                dimension: .lifecycle,
                importance: .key,
                operationID: operationID
            )
        } catch {
            phase = .failed
            errorMessage = error.localizedDescription
            IOSRuntimeTrace.event(
                domain: "auth",
                event: "authSignInFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ]
                ),
                dimension: .lifecycle,
                importance: .essential,
                operationID: operationID
            )
        }
    }

    func activateProfileOnce() async {
        guard session != nil else {
            phase = .signedOut
            return
        }
        guard profileInitialRefreshGeneration != ownerGeneration else {
            IOSRuntimeTrace.decision(
                domain: "auth",
                event: "profileActivationSkipped",
                payload: [
                    "reason": "alreadyActivated",
                    "ownerGeneration": .integer(Int64(ownerGeneration)),
                ],
                dimension: .lifecycle,
                importance: .debug
            )
            return
        }
        profileInitialRefreshGeneration = ownerGeneration
        await refreshProfile(reason: .initialActivation)
    }

    func refreshProfile(
        reason: AuthProfileRefreshReason = .manualPull
    ) async {
        guard session != nil else {
            phase = .signedOut
            IOSRuntimeTrace.decision(
                domain: "auth",
                event: "profileRefreshSkipped",
                payload: ["reason": "signedOut"],
                dimension: .lifecycle,
                importance: .debug
            )
            return
        }
        if let profileRefreshTask {
            IOSRuntimeTrace.decision(
                domain: "auth",
                event: "profileRefreshCoalesced",
                payload: ["reason": .string(reason.rawValue)],
                dimension: .lifecycle,
                importance: .debug
            )
            await profileRefreshTask.value
            return
        }
        let taskID = UUID()
        let task = Task { @MainActor [weak self] in
            guard let self else { return }
            await self.performRefreshProfile(reason: reason)
        }
        profileRefreshTask = task
        profileRefreshTaskID = taskID
        await task.value
        if profileRefreshTaskID == taskID {
            profileRefreshTask = nil
            profileRefreshTaskID = nil
        }
    }

    private func performRefreshProfile(reason: AuthProfileRefreshReason) async {
        guard let session else { return }
        let operationID = UUID().uuidString
        let startedAt = Date()
        let requestGeneration = ownerGeneration
        let requestToken = session.webTokenJSON
        phase = .refreshing
        errorMessage = nil
        IOSRuntimeTrace.event(
            domain: "auth",
            event: "profileRefreshStarted",
            payload: [
                "reason": .string(reason.rawValue),
                "ownerGeneration": .integer(Int64(requestGeneration)),
            ],
            dimension: .network,
            importance: .debug,
            operationID: operationID
        )
        do {
            let loadedProfile = try await client.loadProfile(webTokenJSON: requestToken)
            guard requestGeneration == ownerGeneration else {
                IOSRuntimeTrace.decision(
                    domain: "auth",
                    event: "profileRefreshDiscarded",
                    payload: ["reason": "ownerChanged"],
                    dimension: .lifecycle,
                    importance: .debug,
                    operationID: operationID
                )
                return
            }
            profile = loadedProfile
            phase = .signedIn
            IOSRuntimeTrace.state(
                domain: "auth",
                event: "profileRefreshSucceeded",
                payload: [
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                ],
                dimension: .network,
                importance: .debug,
                operationID: operationID
            )
        } catch {
            guard requestGeneration == ownerGeneration else { return }
            phase = .signedIn
            errorMessage = error.localizedDescription
            IOSRuntimeTrace.event(
                domain: "auth",
                event: "profileRefreshFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    ]
                ),
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
        }
    }

    func prepareCloudAccess() async throws -> PreparedCloudAccess {
        guard let session else {
            IOSRuntimeTrace.decision(
                domain: "auth",
                event: "cloudAccessBoundaryFailed",
                payload: ["errorKind": "signedOut"],
                dimension: .network,
                importance: .essential
            )
            throw AuthStoreError.signedOut
        }
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "auth",
            event: "cloudAccessBoundaryStarted",
            payload: ["appLevel": .integer(Int64(session.appLevel))],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
        do {
            let result = try await cloudClient.prepareAccess(
                refreshToken: session.refreshToken,
                seedJSON: session.seedJSON,
                forceRegionIP: settings.cloudRegionPreset.forceRegionIP
            )
            let refreshed = StoredAuthSession(
                bridgeSession: result.authSession,
                cloudAccountID: result.accountID,
                cloudRegionHost: result.regionHost
            )
            try await keychain.save(refreshed)
            self.session = refreshed
            IOSRuntimeTrace.event(
                domain: "auth",
                event: "cloudAccessBoundarySucceeded",
                payload: [
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    "sessionPersisted": true,
                    "appLevel": .integer(Int64(result.authSession.appLevel)),
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            return result
        } catch {
            IOSRuntimeTrace.event(
                domain: "auth",
                event: "cloudAccessBoundaryFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                        "appLevel": .integer(Int64(session.appLevel)),
                    ]
                ),
                dimension: .network,
                importance: .essential,
                operationID: operationID
            )
            throw error
        }
    }

    func prepareHomeAccess() async throws -> PreparedHomeAccess {
        guard let session else {
            IOSRuntimeTrace.decision(
                domain: "auth",
                event: "homeAccessBoundaryFailed",
                payload: ["errorKind": "signedOut"],
                dimension: .network,
                importance: .essential
            )
            throw AuthStoreError.signedOut
        }
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "auth",
            event: "homeAccessBoundaryStarted",
            payload: ["appLevel": .integer(Int64(session.appLevel))],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
        do {
            let result = try await cloudClient.prepareHomeAccess(
                refreshToken: session.refreshToken,
                seedJSON: session.seedJSON,
                forceRegionIP: settings.cloudRegionPreset.forceRegionIP
            )
            let refreshed = StoredAuthSession(
                bridgeSession: result.authSession,
                cloudAccountID: session.cloudAccountID,
                cloudRegionHost: session.cloudRegionHost
            )
            try await keychain.save(refreshed)
            self.session = refreshed
            IOSRuntimeTrace.event(
                domain: "auth",
                event: "homeAccessBoundarySucceeded",
                payload: [
                    "elapsedMs": .integer(
                        Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))
                    ),
                    "sessionPersisted": true,
                    "appLevel": .integer(Int64(result.authSession.appLevel)),
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            return result
        } catch {
            IOSRuntimeTrace.event(
                domain: "auth",
                event: "homeAccessBoundaryFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: [
                        "elapsedMs": .integer(
                            Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))
                        ),
                        "appLevel": .integer(Int64(session.appLevel)),
                    ]
                ),
                dimension: .network,
                importance: .essential,
                operationID: operationID
            )
            throw error
        }
    }

    func retry() async {
        if session != nil {
            await refreshProfile(reason: .manualRetry)
        } else {
            await signIn()
        }
    }

    func refreshForRegionChange() async -> Bool {
        guard let session else { return false }
        IOSRuntimeTrace.event(
            domain: "auth",
            event: "cloudRegionSessionRefreshStarted",
            payload: [
                "preset": .string(settings.cloudRegionPreset.rawValue),
                "forceRegionApplied": .bool(!settings.cloudRegionPreset.forceRegionIP.isEmpty),
            ],
            dimension: .network,
            importance: .key
        )
        phase = .refreshing
        let renewed = await refreshSession(
            session,
            preserveCloudScope: false,
            markSignedIn: false
        )
        ownerGeneration &+= 1
        profileInitialRefreshGeneration = nil
        phase = .signedIn
        return renewed
    }

    func signOut() async {
        IOSRuntimeTrace.event(
            domain: "auth",
            event: "authSignOutStarted",
            payload: ["hadSession": .bool(session != nil)],
            dimension: .lifecycle,
            importance: .key
        )
        webAuthentication.cancel()
        profileRefreshTask?.cancel()
        profileRefreshTask = nil
        profileRefreshTaskID = nil
        do {
            try await keychain.delete()
        } catch {
            errorMessage = error.localizedDescription
        }
        session = nil
        profile = nil
        ownerGeneration &+= 1
        profileInitialRefreshGeneration = nil
        phase = .signedOut
        IOSRuntimeTrace.state(
            domain: "auth",
            event: "authSignedOut",
            payload: [:],
            dimension: .lifecycle,
            importance: .key
        )
    }

    @discardableResult
    private func refreshSession(
        _ stored: StoredAuthSession,
        preserveCloudScope: Bool = true,
        markSignedIn: Bool = true
    ) async -> Bool {
        let operationID = UUID().uuidString
        let startedAt = Date()
        phase = .refreshing
        errorMessage = nil
        IOSRuntimeTrace.event(
            domain: "auth",
            event: "authSessionRenewStarted",
            payload: [:],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )

        do {
            let bridgeSession = try await client.renewLogin(
                refreshToken: stored.refreshToken,
                seedJSON: stored.seedJSON,
                forceRegionIP: settings.cloudRegionPreset.forceRegionIP
            )
            let refreshed = StoredAuthSession(
                bridgeSession: bridgeSession,
                cloudAccountID: preserveCloudScope ? stored.cloudAccountID : nil,
                cloudRegionHost: preserveCloudScope ? stored.cloudRegionHost : nil
            )
            try await keychain.save(refreshed)
            session = refreshed
            if markSignedIn {
                phase = .signedIn
            }
            IOSRuntimeTrace.state(
                domain: "auth",
                event: "authSessionRenewSucceeded",
                payload: [
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    "appLevel": .integer(Int64(refreshed.appLevel)),
                    "regionPreset": .string(settings.cloudRegionPreset.rawValue),
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            return true
        } catch {
            errorMessage = CloudLibraryDiagnostics.safeError(error)
            IOSRuntimeTrace.decision(
                domain: "auth",
                event: "authSessionRenewFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: ["fallback": "storedWebToken"]
                ),
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            session = stored
            if markSignedIn {
                phase = .signedIn
            }
            IOSRuntimeTrace.state(
                domain: "auth",
                event: "authSessionFallbackSucceeded",
                payload: [
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    "fallback": "storedCredentials",
                ],
                dimension: .lifecycle,
                importance: .key,
                operationID: operationID
            )
            return false
        }
    }
}

private enum AuthStoreError: LocalizedError {
    case signedOut

    var errorDescription: String? {
        "Xbox 会话已经结束"
    }
}
