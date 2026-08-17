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
    private let credentialCoordinator = AuthCredentialCoordinator()
    private var recoveryAction: AuthRecoveryAction = .signIn
    private var restored = false
    private var isSigningOut = false
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
        let requestRevision = credentialCoordinator.currentRevision
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
                guard credentialCoordinator.isCurrent(requestRevision) else { return }
                phase = .signedOut
                recoveryAction = .signIn
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
            guard credentialCoordinator.isCurrent(requestRevision) else { return }
            profile = nil
            IOSRuntimeTrace.decision(
                domain: "auth",
                event: "authRestoreSessionFound",
                payload: ["action": "renewCredentials"],
                dimension: .lifecycle,
                importance: .key,
                operationID: operationID
            )
            let disposition = await refreshSession(
                stored,
                markSignedIn: false,
                expectedRevision: requestRevision
            )
            guard disposition.establishesSignedInState else { return }
            ownerGeneration &+= 1
            profileInitialRefreshGeneration = nil
            phase = .signedIn
            recoveryAction = .refreshProfile
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
            guard credentialCoordinator.isCurrent(requestRevision) else { return }
            phase = .failed
            recoveryAction = .signIn
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
        let requestRevision = credentialCoordinator.currentRevision
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
            _ = try await credentialCoordinator.perform(
                expectedRevision: requestRevision,
                advancesRevision: true
            ) { [weak self] revision in
                guard let self else { throw AuthStoreError.staleOperation }
                try await self.keychain.save(stored)
                guard self.credentialCoordinator.isCurrent(revision) else {
                    throw AuthStoreError.staleOperation
                }
                self.session = stored
                self.ownerGeneration &+= 1
                self.profileInitialRefreshGeneration = nil
                self.profile = nil
                self.phase = .signedIn
                self.recoveryAction = .refreshProfile
            }
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
            guard credentialCoordinator.isCurrent(requestRevision) else { return }
            phase = session == nil ? .signedOut : .failed
            recoveryAction = .signIn
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
            guard credentialCoordinator.isCurrent(requestRevision), !AuthStoreError.isStale(error)
            else { return }
            phase = .failed
            recoveryAction = .signIn
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
            recoveryAction = .refreshProfile
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
            recoveryAction = .refreshProfile
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
        guard session != nil, !isSigningOut else {
            IOSRuntimeTrace.decision(
                domain: "auth",
                event: "cloudAccessBoundaryFailed",
                payload: ["errorKind": "signedOut"],
                dimension: .network,
                importance: .essential
            )
            throw AuthStoreError.signedOut
        }
        let requestOwnerGeneration = ownerGeneration
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "auth",
            event: "cloudAccessBoundaryStarted",
            payload: ["ownerGeneration": .integer(Int64(requestOwnerGeneration))],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
        do {
            let result = try await performAccessPreparation(
                ownerGeneration: requestOwnerGeneration,
                prepare: { [cloudClient, settings] currentSession in
                    try await cloudClient.prepareAccess(
                        refreshToken: currentSession.refreshToken,
                        seedJSON: currentSession.seedJSON,
                        forceRegionIP: settings.cloudRegionPreset.forceRegionIP
                    )
                },
                refreshedSession: { result, _ in
                    StoredAuthSession(
                        bridgeSession: result.authSession,
                        cloudAccountID: result.accountID,
                        cloudRegionHost: result.regionHost
                    )
                },
                accessHandle: { $0.handle }
            )
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
                        "ownerGeneration": .integer(Int64(requestOwnerGeneration)),
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
        guard session != nil, !isSigningOut else {
            IOSRuntimeTrace.decision(
                domain: "auth",
                event: "homeAccessBoundaryFailed",
                payload: ["errorKind": "signedOut"],
                dimension: .network,
                importance: .essential
            )
            throw AuthStoreError.signedOut
        }
        let requestOwnerGeneration = ownerGeneration
        let operationID = UUID().uuidString
        let startedAt = Date()
        IOSRuntimeTrace.event(
            domain: "auth",
            event: "homeAccessBoundaryStarted",
            payload: ["ownerGeneration": .integer(Int64(requestOwnerGeneration))],
            dimension: .network,
            importance: .key,
            operationID: operationID
        )
        do {
            let result = try await performAccessPreparation(
                ownerGeneration: requestOwnerGeneration,
                prepare: { [cloudClient, settings] currentSession in
                    try await cloudClient.prepareHomeAccess(
                        refreshToken: currentSession.refreshToken,
                        seedJSON: currentSession.seedJSON,
                        forceRegionIP: settings.cloudRegionPreset.forceRegionIP
                    )
                },
                refreshedSession: { result, currentSession in
                    StoredAuthSession(
                        bridgeSession: result.authSession,
                        cloudAccountID: currentSession.cloudAccountID,
                        cloudRegionHost: currentSession.cloudRegionHost
                    )
                },
                accessHandle: { $0.handle }
            )
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
                        "ownerGeneration": .integer(Int64(requestOwnerGeneration)),
                    ]
                ),
                dimension: .network,
                importance: .essential,
                operationID: operationID
            )
            throw error
        }
    }

    private func performAccessPreparation<Result: Sendable>(
        ownerGeneration requestOwnerGeneration: UInt64,
        prepare: @escaping @MainActor (StoredAuthSession) async throws -> Result,
        refreshedSession: @escaping @MainActor (Result, StoredAuthSession) -> StoredAuthSession,
        accessHandle: @escaping @MainActor (Result) -> String
    ) async throws -> Result {
        let commit = try await credentialCoordinator.perform(
            advancesRevision: true
        ) { [weak self] revision in
            guard let self,
                  !self.isSigningOut,
                  self.ownerGeneration == requestOwnerGeneration,
                  let currentSession = self.session
            else {
                throw AuthStoreError.staleOperation
            }
            let result = try await prepare(currentSession)
            do {
                guard self.canCommitAccess(
                    revision: revision,
                    ownerGeneration: requestOwnerGeneration
                ) else {
                    throw AuthStoreError.staleOperation
                }
                let refreshed = refreshedSession(result, currentSession)
                try await self.keychain.save(refreshed)
                guard self.canCommitAccess(
                    revision: revision,
                    ownerGeneration: requestOwnerGeneration
                ) else {
                    throw AuthStoreError.staleOperation
                }
                self.session = refreshed
                return result
            } catch {
                await self.cloudClient.releaseAccess(handle: accessHandle(result))
                throw error
            }
        }
        return commit.value
    }

    private func canCommitAccess(revision: UInt64, ownerGeneration: UInt64) -> Bool {
        credentialCoordinator.isCurrent(revision)
            && !isSigningOut
            && self.ownerGeneration == ownerGeneration
    }

    func retry() async {
        switch recoveryAction {
        case .signIn:
            await signIn()
        case .signOut:
            await signOut()
        case .refreshProfile where session != nil:
            await refreshProfile(reason: .manualRetry)
        case .refreshProfile:
            await signIn()
        }
    }

    func refreshForRegionChange() async -> Bool {
        guard let session, !isSigningOut else { return false }
        let requestRevision = credentialCoordinator.currentRevision
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
        let disposition = await refreshSession(
            session,
            preserveCloudScope: false,
            markSignedIn: false,
            expectedRevision: requestRevision
        )
        guard disposition.establishesSignedInState else { return false }
        ownerGeneration &+= 1
        profileInitialRefreshGeneration = nil
        phase = .signedIn
        return disposition == .renewed
    }

    func signOut() async {
        guard !isSigningOut else {
            IOSRuntimeTrace.decision(
                domain: "auth",
                event: "authSignOutSkipped",
                payload: ["reason": "alreadySigningOut"],
                dimension: .lifecycle,
                importance: .debug
            )
            return
        }
        isSigningOut = true
        defer { isSigningOut = false }
        let signOutRevision = credentialCoordinator.invalidate()
        ownerGeneration &+= 1
        profileInitialRefreshGeneration = nil
        phase = .refreshing
        errorMessage = nil
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
            _ = try await credentialCoordinator.perform(
                expectedRevision: signOutRevision,
                advancesRevision: true
            ) { [weak self] revision in
                guard let self else { throw AuthStoreError.staleOperation }
                try await self.keychain.delete()
                guard self.credentialCoordinator.isCurrent(revision) else {
                    throw AuthStoreError.staleOperation
                }
                self.session = nil
                self.profile = nil
                self.phase = .signedOut
                self.recoveryAction = .signIn
            }
        } catch {
            guard credentialCoordinator.isCurrent(signOutRevision), !AuthStoreError.isStale(error)
            else { return }
            errorMessage = error.localizedDescription
            phase = .failed
            recoveryAction = .signOut
            IOSRuntimeTrace.event(
                domain: "auth",
                event: "authSignOutFailed",
                payload: CloudLibraryDiagnostics.errorPayload(
                    error,
                    extra: ["sessionRetained": .bool(session != nil)]
                ),
                dimension: .lifecycle,
                importance: .essential
            )
            return
        }
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
        markSignedIn: Bool = true,
        expectedRevision: UInt64? = nil
    ) async -> SessionRefreshDisposition {
        let operationID = UUID().uuidString
        let startedAt = Date()
        let requestRevision = expectedRevision ?? credentialCoordinator.currentRevision
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
            let commit = try await credentialCoordinator.perform(
                expectedRevision: requestRevision,
                advancesRevision: true
            ) { [weak self] revision -> SessionRefreshDisposition in
                guard let self else { throw AuthStoreError.staleOperation }
                do {
                    let bridgeSession = try await self.client.renewLogin(
                        refreshToken: stored.refreshToken,
                        seedJSON: stored.seedJSON,
                        forceRegionIP: self.settings.cloudRegionPreset.forceRegionIP
                    )
                    guard self.credentialCoordinator.isCurrent(revision) else {
                        throw AuthStoreError.staleOperation
                    }
                    let refreshed = StoredAuthSession(
                        bridgeSession: bridgeSession,
                        cloudAccountID: preserveCloudScope ? stored.cloudAccountID : nil,
                        cloudRegionHost: preserveCloudScope ? stored.cloudRegionHost : nil
                    )
                    try await self.keychain.save(refreshed)
                    guard self.credentialCoordinator.isCurrent(revision) else {
                        throw AuthStoreError.staleOperation
                    }
                    self.session = refreshed
                    if markSignedIn {
                        self.phase = .signedIn
                    }
                    self.recoveryAction = .refreshProfile
                    return .renewed
                } catch {
                    guard self.credentialCoordinator.isCurrent(revision),
                          !AuthStoreError.isStale(error)
                    else {
                        throw AuthStoreError.staleOperation
                    }
                    self.errorMessage = CloudLibraryDiagnostics.safeError(error)
                    let failureDisposition = AuthRenewalFailureDisposition(error: error)
                    IOSRuntimeTrace.decision(
                        domain: "auth",
                        event: "authSessionRenewFailed",
                        payload: CloudLibraryDiagnostics.errorPayload(
                            error,
                            extra: [
                                "disposition": .string(failureDisposition.rawValue),
                                "fallback": .string(
                                    failureDisposition == .retryable
                                        ? "storedWebToken"
                                        : "rejected"
                                ),
                            ]
                        ),
                        dimension: .network,
                        importance: .key,
                        operationID: operationID
                    )
                    guard failureDisposition == .retryable else {
                        self.phase = .failed
                        self.recoveryAction = .signIn
                        return .rejected
                    }
                    self.session = stored
                    if markSignedIn {
                        self.phase = .signedIn
                    }
                    self.recoveryAction = .refreshProfile
                    return .retryableFallback
                }
            }
            let disposition = commit.value
            guard disposition == .renewed else {
                if disposition == .retryableFallback {
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
                }
                return disposition
            }
            IOSRuntimeTrace.state(
                domain: "auth",
                event: "authSessionRenewSucceeded",
                payload: [
                    "elapsedMs": .integer(Int64(CloudLibraryDiagnostics.elapsedMilliseconds(since: startedAt))),
                    "appLevel": .integer(Int64(session?.appLevel ?? stored.appLevel)),
                    "regionPreset": .string(settings.cloudRegionPreset.rawValue),
                ],
                dimension: .network,
                importance: .key,
                operationID: operationID
            )
            return .renewed
        } catch {
            guard !AuthStoreError.isStale(error) else { return .stale }
            errorMessage = CloudLibraryDiagnostics.safeError(error)
            phase = .failed
            recoveryAction = .signIn
            return .rejected
        }
    }
}
