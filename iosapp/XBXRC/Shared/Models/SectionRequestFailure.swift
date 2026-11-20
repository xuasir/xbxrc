import Foundation

enum SectionRequestAttempt: String, Sendable { case initial, refresh, nextPage }

struct SectionRequestFailure: Error, Equatable, Sendable {
    enum Kind: String, Sendable {
        case network, timeout, authentication, accessRestricted, rateLimited, serviceUnavailable
        case invalidResponse, repeatedContinuation, unknown
    }
    let kind: Kind
    var retryAt: Date? = nil

    init(kind: Kind, retryAt: Date? = nil) {
        self.kind = kind
        self.retryAt = retryAt
    }
    init(_ failure: XboxQueryFailure) {
        switch failure.kind {
        case .network: kind = .network
        case .timeout: kind = .timeout
        case .authentication: kind = .authentication
        case .accessRestricted: kind = .accessRestricted
        case .rateLimited: kind = .rateLimited
        case .serviceUnavailable: kind = .serviceUnavailable
        case .invalidResponse: kind = .invalidResponse
        case .unknown: kind = .unknown
        }
        retryAt = failure.retryAfterSeconds.map { Date().addingTimeInterval(Double($0)) }
    }
    init(error: any Error) {
        if let failure = error as? SectionRequestFailure {
            self = failure
            return
        }
        if let bridge = error as? XboxBridgeError, case .Query(let failure) = bridge {
            self.init(failure)
            return
        }
        if let bridge = error as? XboxBridgeError {
            switch bridge {
            case .Authentication: self.init(kind: .authentication)
            case .RetryableAuthentication: self.init(kind: .serviceUnavailable)
            case .InvalidData: self.init(kind: .invalidResponse)
            default: self.init(kind: .unknown)
            }
            return
        }
        if let native = error as? XboxNativeQueryError, case .repeatedContinuation = native {
            self.init(kind: .repeatedContinuation)
            return
        }
        if let url = error as? URLError {
            self.init(kind: url.code == .timedOut ? .timeout : .network)
            return
        }
        self.init(kind: .unknown)
    }
    static func isCancellation(_ error: any Error) -> Bool {
        error is CancellationError || (error as? URLError)?.code == .cancelled
            || AuthStoreError.isStale(error)
    }
    var message: String {
        switch kind {
        case .network: "暂时无法连接 Xbox 服务，请检查网络后重试。"
        case .timeout: "Xbox 服务响应较慢，请稍后重试。"
        case .authentication: "登录状态需要更新，请重新登录后继续加载。"
        case .accessRestricted: "当前账户暂时无法访问这部分资料，请检查账户和访问条件。"
        case .rateLimited: "请求较频繁，请稍后再试。"
        case .serviceUnavailable: "Xbox 服务暂时无法提供这部分资料，请稍后重试。"
        case .invalidResponse, .unknown: "这部分资料暂时无法载入，请稍后重试。"
        case .repeatedContinuation: "这份列表暂时无法继续加载，请刷新列表后重试。"
        }
    }
    var actionTitle: String {
        switch kind {
        case .authentication: "重新登录"
        case .accessRestricted: "再次检查"
        case .repeatedContinuation: "刷新列表"
        default: "重试"
        }
    }
}

struct SectionRequestStatus: Sendable {
    var phase: DataLoadPhase = .idle
    var isBusy = false
    var hasSnapshot = false
    var updatedAt: Date?
    var attempt: SectionRequestAttempt = .initial
    var failure: SectionRequestFailure?
}

extension XboxNativeDataState {
    var hasSnapshot: Bool { updatedAt != nil || phase == .loaded || !items.isEmpty }
    var requestStatus: SectionRequestStatus {
        SectionRequestStatus(
            phase: phase, isBusy: isRefreshing, hasSnapshot: hasSnapshot,
            updatedAt: updatedAt, attempt: requestAttempt,
            failure: failure ?? (errorMessage == nil ? nil : SectionRequestFailure(kind: .unknown)))
    }
    mutating func beginRequest(nextPage: Bool = false) {
        requestAttempt = nextPage ? .nextPage : hasSnapshot ? .refresh : .initial
        isRefreshing = true
        if !hasSnapshot { phase = .loading }
        failure = nil
        errorMessage = nil
    }
    mutating func failRequest(_ error: any Error) {
        phase =
            hasSnapshot ? .loaded : SectionRequestFailure.isCancellation(error) ? .idle : .failed
        failure =
            SectionRequestFailure.isCancellation(error) ? nil : SectionRequestFailure(error: error)
        errorMessage = failure?.message
        isRefreshing = false
    }
}

extension XboxSingleDataState {
    var hasSnapshot: Bool { updatedAt != nil || phase == .loaded || value != nil }
    var requestStatus: SectionRequestStatus {
        SectionRequestStatus(
            phase: phase, isBusy: isRefreshing, hasSnapshot: hasSnapshot,
            updatedAt: updatedAt, attempt: requestAttempt,
            failure: failure ?? (errorMessage == nil ? nil : SectionRequestFailure(kind: .unknown)))
    }
    mutating func beginRequest() {
        requestAttempt = hasSnapshot ? .refresh : .initial
        isRefreshing = true
        if !hasSnapshot { phase = .loading }
        failure = nil
        errorMessage = nil
    }
    mutating func failRequest(_ error: any Error) {
        phase =
            hasSnapshot ? .loaded : SectionRequestFailure.isCancellation(error) ? .idle : .failed
        failure =
            SectionRequestFailure.isCancellation(error) ? nil : SectionRequestFailure(error: error)
        errorMessage = failure?.message
        isRefreshing = false
    }
}
