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
    @Published private(set) var needsDataSignIn = false
    @Published private(set) var profileSectionStates: [XboxProfileSource: XboxSingleDataState<XboxProfile>] = [:]
    private var profileSourceRequests: [XboxProfileSource: UUID] = [:]
    private var dataRenewalTask: Task<String?, Never>?
    private var dataRenewalFailure: SectionRequestFailure?


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
            profileSectionStates = [:]
            profileSourceRequests = [:]
            needsDataSignIn = false
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
            errorMessage = SectionRequestFailure(error: error).message
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
                self.profileSectionStates = [:]
                self.profileSourceRequests = [:]
                self.needsDataSignIn = false
                self.dataRenewalTask?.cancel()
                self.dataRenewalTask = nil
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
            errorMessage = SectionRequestFailure(error: error).message
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
        guard session != nil else { return }
        phase = .refreshing
        let owner = ownerGeneration
        await withTaskGroup(of: Void.self) { group in
            for source in [XboxProfileSource.identity, .social, .friends, .presence] {
                group.addTask { await self.refreshProfileSource(source) }
            }
        }
        if owner == ownerGeneration && !needsDataSignIn { phase = .signedIn }
    }

    func refreshProfileSource(_ source: XboxProfileSource) async {
        guard let session, profileSourceRequests[source] == nil else { return }
        let request = UUID()
        let owner = ownerGeneration
        profileSourceRequests[source] = request
        var state = profileSectionStates[source] ?? XboxSingleDataState()
        state.beginRequest()
        profileSectionStates[source] = state
        do {
            let patch: XboxProfile
            do { patch = try await client.loadProfileSource(webTokenJSON: session.webTokenJSON, source: source) }
            catch {
                guard SectionRequestFailure(error: error).kind == .authentication,
                    let token = try await recoverDataToken(rejectedToken: session.webTokenJSON) else { throw error }
                patch = try await client.loadProfileSource(webTokenJSON: token, source: source)
            }
            try Task.checkCancellation()
            guard owner == ownerGeneration, profileSourceRequests[source] == request else { return }
            state.value = patch; state.phase = .loaded; state.updatedAt = .now
            mergeProfile(patch, source: source)
        } catch {
            guard owner == ownerGeneration, profileSourceRequests[source] == request else { return }
            state.failRequest(error)
            if state.failure?.kind == .authentication { requireDataSignIn() }
        }
        guard owner == ownerGeneration, profileSourceRequests[source] == request else { return }
        state.isRefreshing = false
        profileSectionStates[source] = state
        profileSourceRequests[source] = nil
    }

    func requireDataSignIn() {
        needsDataSignIn = true
        recoveryAction = .signIn
    }

    func recoverDataToken(rejectedToken: String) async throws -> String? {
        guard let stored = session else { return nil }
        if stored.webTokenJSON != rejectedToken { return stored.webTokenJSON }
        if let dataRenewalTask {
            let token = await dataRenewalTask.value
            if let dataRenewalFailure { throw dataRenewalFailure }
            return token
        }
        dataRenewalFailure = nil
        let owner = ownerGeneration
        let task = Task { @MainActor [weak self] () -> String? in
            guard let self else { return nil }
            let disposition = await self.refreshSession(stored, forceCoreRefresh: true)
            guard owner == self.ownerGeneration else { return nil }
            if disposition == .renewed {
                self.needsDataSignIn = false
                self.dataRenewalFailure = nil
                return self.session?.webTokenJSON
            }
            if disposition == .rejected {
                self.needsDataSignIn = true
                self.recoveryAction = .signIn
                self.dataRenewalFailure = SectionRequestFailure(kind: .authentication)
            } else if disposition == .stale { return nil }
            else { self.dataRenewalFailure = self.dataRenewalFailure ?? SectionRequestFailure(kind: .serviceUnavailable) }
            return nil
        }
        dataRenewalTask = task
        let token = await task.value
        if owner == ownerGeneration { dataRenewalTask = nil }
        else { throw CancellationError() }
        if let dataRenewalFailure { throw dataRenewalFailure }
        return token
    }

    private func mergeProfile(_ patch: XboxProfile, source: XboxProfileSource) {
        let old = profile
        profile = XboxProfile(
            xuid: patch.xuid ?? old?.xuid,
            gamertag: source == .identity ? patch.gamertag : old?.gamertag ?? "",
            displayName: source == .identity ? patch.displayName : old?.displayName ?? "",
            gamerScore: source == .identity ? patch.gamerScore : old?.gamerScore ?? "",
            displayPictureUrl: source == .identity ? patch.displayPictureUrl : old?.displayPictureUrl ?? "",
            presenceState: source == .presence ? patch.presenceState : old?.presenceState,
            presenceDevice: source == .presence ? patch.presenceDevice : old?.presenceDevice,
            currentTitleName: source == .presence ? patch.currentTitleName : old?.currentTitleName,
            richPresence: source == .presence ? patch.richPresence : old?.richPresence,
            followersCount: source == .social ? patch.followersCount : old?.followersCount,
            followingCount: source == .social ? patch.followingCount : old?.followingCount,
            friendCount: source == .friends ? patch.friendCount : old?.friendCount)
    }

    func prepareCloudAccess(startup: XboxStreamStartup? = nil) async throws -> PreparedCloudAccess {
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
        let authProvider = makeStreamingAuthProvider(ownerGeneration: requestOwnerGeneration)
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
                prepare: { [cloudClient, settings] currentSession, checkpoint in
                    try await cloudClient.prepareAccess(
                        session: currentSession,
                        forceRegionIP: settings.cloudRegionPreset.forceRegionIP,
                        checkpoint: checkpoint,
                        authProvider: authProvider,
                        startup: startup
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

    func prepareHomeAccess(startup: XboxStreamStartup? = nil) async throws -> PreparedHomeAccess {
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
        let authProvider = makeStreamingAuthProvider(ownerGeneration: requestOwnerGeneration)
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
                prepare: { [cloudClient, settings] currentSession, checkpoint in
                    try await cloudClient.prepareHomeAccess(
                        session: currentSession,
                        forceRegionIP: settings.cloudRegionPreset.forceRegionIP,
                        checkpoint: checkpoint,
                        authProvider: authProvider,
                        startup: startup
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

    func persistStreamingAuthUpdate(
        _ update: XboxStreamingAuthUpdate,
        expectedOwnerGeneration: UInt64
    ) async throws -> StreamingAuthCommitResult {
        let commit = try await credentialCoordinator.perform(advancesRevision: true) { [weak self] revision in
            guard let self, self.canCommitAccess(revision: revision, ownerGeneration: expectedOwnerGeneration),
                let current = self.session,
                Self.streamingAccountID(current) == update.accountId
            else { throw AuthStoreError.staleOperation }
            if current.bridgeSession == update.authSession {
                return StreamingAuthCommitResult.committed
            }
            // 其他认证入口已提交更近的凭据时，会话沿同一账户同步它。
            if current.refreshToken != update.previousRefreshToken
                && current.refreshToken != update.authSession.refreshToken
            {
                return .synchronize(current.bridgeSession)
            }
            let refreshed = StoredAuthSession(bridgeSession: update.authSession,
                cloudAccountID: current.cloudAccountID, cloudRegionHost: current.cloudRegionHost)
            try await self.keychain.save(refreshed)
            guard self.canCommitAccess(revision: revision, ownerGeneration: expectedOwnerGeneration) else {
                throw AuthStoreError.staleOperation
            }
            self.session = refreshed
            return .committed
        }
        return commit.value
    }

    private static func streamingAccountID(_ session: StoredAuthSession) -> String? {
        guard let data = session.webTokenJSON.data(using: .utf8),
            let token = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return session.cloudAccountID }
        let root = token["data"] as? [String: Any] ?? token
        guard let claims = root["DisplayClaims"] as? [String: Any],
            let users = claims["xui"] as? [[String: Any]], let user = users.first
        else { return session.cloudAccountID }
        return user["xid"] as? String ?? session.cloudAccountID ?? user["uhs"] as? String
    }

    private func performAccessPreparation<Result: Sendable>(
        ownerGeneration requestOwnerGeneration: UInt64,
        prepare:
            @escaping @MainActor (StoredAuthSession, @escaping AuthCredentialCheckpoint)
            async throws ->
            Result,
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
            let checkpoint = self.makeCredentialCheckpoint(
                revision: revision,
                ownerGeneration: requestOwnerGeneration, fallback: currentSession)
            let result = try await prepare(currentSession, checkpoint)
            do {
                guard self.canCommitAccess(
                    revision: revision,
                    ownerGeneration: requestOwnerGeneration
                ) else {
                    throw AuthStoreError.staleOperation
                }
                try Task.checkCancellation()
                let refreshed = refreshedSession(result, self.session ?? currentSession)
                if self.session != refreshed { try await self.keychain.save(refreshed) }
                guard
                    self.canCommitAccess(
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

    private func makeCredentialCheckpoint(
        revision: UInt64, ownerGeneration: UInt64, fallback: StoredAuthSession,
        preserveCloudScope: Bool = true
    ) -> AuthCredentialCheckpoint {
        { [weak self] update in
            guard let self,
                self.canCommitAccess(revision: revision, ownerGeneration: ownerGeneration)
            else {
                throw AuthStoreError.staleOperation
            }
            let current = self.session ?? fallback
            let refreshed = StoredAuthSession(
                bridgeSession: update.authSession,
                cloudAccountID: preserveCloudScope ? current.cloudAccountID : nil,
                cloudRegionHost: preserveCloudScope ? current.cloudRegionHost : nil)
            if current != refreshed { try await self.keychain.save(refreshed) }
            guard self.canCommitAccess(revision: revision, ownerGeneration: ownerGeneration) else {
                throw AuthStoreError.staleOperation
            }
            // 核心轮换在当前认证事务中保存，继续使用同一个 revision。
            self.session = refreshed
            IOSRuntimeTrace.event(
                domain: "auth", event: "credentialCheckpointCommitted",
                payload: [
                    "coreOnly": .bool(update.coreOnly),
                    "hasTokenCache": .bool(refreshed.tokenCacheJSON != nil),
                ],
                dimension: .lifecycle, importance: .key)
        }
    }

    private func makeStreamingAuthProvider(ownerGeneration: UInt64) -> RustStreamingAuthProvider {
        RustStreamingAuthProvider { [weak self] target, rejected in
            guard let self else { throw AuthStoreError.staleOperation }
            return try await self.ensureStreamingCredentials(
                target: target, rejectedToken: rejected,
                ownerGeneration: ownerGeneration)
        }
    }

    private func ensureStreamingCredentials(
        target: String, rejectedToken: String?, ownerGeneration: UInt64
    ) async throws -> AuthSession {
        let scope: XboxAuthScope
        switch target {
        case "web": scope = .web
        case "home": scope = .home
        case "cloud": scope = .cloud
        default: throw XboxBridgeError.InvalidData("streamingCredentialTargetInvalid")
        }
        let commit = try await credentialCoordinator.perform(advancesRevision: true) {
            [weak self] revision in
            guard let self,
                self.canCommitAccess(revision: revision, ownerGeneration: ownerGeneration),
                let current = self.session
            else { throw AuthStoreError.staleOperation }
            let auth = try await self.client.renewLogin(
                session: current,
                forceRegionIP: self.settings.cloudRegionPreset.forceRegionIP, scope: scope,
                forceCoreRefresh: false, rejectedToken: rejectedToken,
                checkpoint: self.makeCredentialCheckpoint(
                    revision: revision,
                    ownerGeneration: ownerGeneration, fallback: current))
            guard self.canCommitAccess(revision: revision, ownerGeneration: ownerGeneration) else {
                throw AuthStoreError.staleOperation
            }
            return auth
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
            scope: .cloud,
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
                self.profileSectionStates = [:]
                self.profileSourceRequests = [:]
                self.needsDataSignIn = false
                self.dataRenewalTask?.cancel()
                self.dataRenewalTask = nil
                self.phase = .signedOut
                self.recoveryAction = .signIn
            }
        } catch {
            guard credentialCoordinator.isCurrent(signOutRevision), !AuthStoreError.isStale(error)
            else { return }
            errorMessage = SectionRequestFailure(error: error).message
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
        scope: XboxAuthScope = .web,
        forceCoreRefresh: Bool = false,
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
                let current = self.session ?? stored
                do {
                    let bridgeSession = try await self.client.renewLogin(
                        session: current,
                        forceRegionIP: self.settings.cloudRegionPreset.forceRegionIP,
                        scope: scope,
                        forceCoreRefresh: forceCoreRefresh,
                        rejectedToken: nil,
                        checkpoint: self.makeCredentialCheckpoint(
                            revision: revision,
                            ownerGeneration: self.ownerGeneration, fallback: current,
                            preserveCloudScope: preserveCloudScope)
                    )
                    guard self.credentialCoordinator.isCurrent(revision) else {
                        throw AuthStoreError.staleOperation
                    }
                    let refreshed = StoredAuthSession(
                        bridgeSession: bridgeSession,
                        cloudAccountID: preserveCloudScope ? stored.cloudAccountID : nil,
                        cloudRegionHost: preserveCloudScope ? stored.cloudRegionHost : nil
                    )
                    if self.session != refreshed { try await self.keychain.save(refreshed) }
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
                    if Task.isCancelled || AuthStoreError.isCancelled(error) {
                        throw CancellationError()
                    }
                    guard self.credentialCoordinator.isCurrent(revision),
                          !AuthStoreError.isStale(error)
                    else {
                        throw AuthStoreError.staleOperation
                    }
                    self.errorMessage = SectionRequestFailure(error: error).message
                    self.dataRenewalFailure = SectionRequestFailure(error: error)
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
                    self.session = self.session ?? current
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
            guard !AuthStoreError.isStale(error), !(error is CancellationError) else {
                return .stale
            }
            errorMessage = SectionRequestFailure(error: error).message
            phase = .failed
            recoveryAction = .signIn
            return .rejected
        }
    }
}
