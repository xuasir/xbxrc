import Foundation

enum SessionRefreshDisposition: Equatable {
    case renewed
    case retryableFallback
    case rejected
    case stale

    var establishesSignedInState: Bool {
        self == .renewed || self == .retryableFallback
    }
}

enum AuthRecoveryAction {
    case signIn
    case signOut
    case refreshProfile
}

enum AuthRenewalFailureDisposition: String {
    case retryable
    case terminal

    init(error: Error) {
        if let urlError = error as? URLError {
            self = Self.retryableURLCodes.contains(urlError.code) ? .retryable : .terminal
            return
        }

        if let bridgeError = error as? XboxBridgeError {
            switch bridgeError {
            case .RetryableAuthentication:
                self = .retryable
            case .InvalidData, .Profile, .Data:
                self = .terminal
            case .Authentication:
                self = .terminal
            }
            return
        }

        let nsError = error as NSError
        if nsError.domain == NSURLErrorDomain {
            let code = URLError.Code(rawValue: nsError.code)
            self = Self.retryableURLCodes.contains(code) ? .retryable : .terminal
            return
        }
        self = .terminal
    }

    private static let retryableURLCodes: Set<URLError.Code> = [
        .timedOut,
        .cannotFindHost,
        .cannotConnectToHost,
        .networkConnectionLost,
        .dnsLookupFailed,
        .notConnectedToInternet,
        .internationalRoamingOff,
        .callIsActive,
        .dataNotAllowed,
        .secureConnectionFailed,
    ]

}

@MainActor
final class AuthCredentialCoordinator {
    struct Commit<Value: Sendable>: Sendable {
        let value: Value
        let revision: UInt64
    }

    private(set) var currentRevision: UInt64 = 0
    private var mutationTail: Task<Void, Never>?

    func isCurrent(_ revision: UInt64) -> Bool {
        currentRevision == revision
    }

    @discardableResult
    func invalidate() -> UInt64 {
        currentRevision &+= 1
        return currentRevision
    }

    func perform<Value: Sendable>(
        expectedRevision: UInt64? = nil,
        advancesRevision: Bool,
        operation: @escaping @MainActor (UInt64) async throws -> Value
    ) async throws -> Commit<Value> {
        let predecessor = mutationTail
        let task = Task { @MainActor [weak self] () throws -> Commit<Value> in
            await predecessor?.value
            guard let self else { throw AuthStoreError.staleOperation }
            if let expectedRevision,
               expectedRevision != self.currentRevision {
                throw AuthStoreError.staleOperation
            }

            let operationRevision = self.currentRevision
            let value = try await operation(operationRevision)
            guard operationRevision == self.currentRevision else {
                throw AuthStoreError.staleOperation
            }
            if advancesRevision {
                self.currentRevision &+= 1
            }
            return Commit(value: value, revision: self.currentRevision)
        }
        mutationTail = Task { @MainActor in
            _ = try? await task.value
        }
        return try await task.value
    }
}

enum AuthStoreError: LocalizedError {
    case signedOut
    case staleOperation

    static func isStale(_ error: Error) -> Bool {
        guard let authError = error as? AuthStoreError else { return false }
        if case .staleOperation = authError { return true }
        return false
    }

    var errorDescription: String? {
        switch self {
        case .signedOut:
            "Xbox 会话已经结束"
        case .staleOperation:
            "认证状态已经更新，请重试"
        }
    }
}
