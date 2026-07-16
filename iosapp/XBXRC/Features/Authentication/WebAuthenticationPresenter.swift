import AuthenticationServices
import UIKit

enum WebAuthenticationError: LocalizedError {
    case invalidAuthorizationURL
    case sessionAlreadyRunning
    case sessionStartFailed

    var errorDescription: String? {
        switch self {
        case .invalidAuthorizationURL:
            "Xbox 登录地址无效"
        case .sessionAlreadyRunning:
            "Xbox 登录正在进行"
        case .sessionStartFailed:
            "无法打开 Xbox 登录页面"
        }
    }
}

@MainActor
protocol WebAuthenticating: AnyObject {
    func authenticate(
        authorizationURL: String,
        prefersEphemeralSession: Bool
    ) async throws -> URL
    func cancel()
}

@MainActor
final class WebAuthenticationPresenter: NSObject, WebAuthenticating,
    ASWebAuthenticationPresentationContextProviding
{
    private static let callbackScheme = "ms-xal-000000004c20a908"
    private var session: ASWebAuthenticationSession?

    func authenticate(
        authorizationURL: String,
        prefersEphemeralSession: Bool
    ) async throws -> URL {
        guard let url = URL(string: authorizationURL) else {
            throw WebAuthenticationError.invalidAuthorizationURL
        }
        guard session == nil else {
            throw WebAuthenticationError.sessionAlreadyRunning
        }

        return try await withCheckedThrowingContinuation { continuation in
            let session = ASWebAuthenticationSession(
                url: url,
                callbackURLScheme: Self.callbackScheme
            ) { [weak self] callbackURL, error in
                Task { @MainActor in
                    self?.session = nil
                    if let callbackURL {
                        continuation.resume(returning: callbackURL)
                    } else {
                        continuation.resume(
                            throwing: error ?? WebAuthenticationError.sessionStartFailed
                        )
                    }
                }
            }
            session.presentationContextProvider = self
            session.prefersEphemeralWebBrowserSession = prefersEphemeralSession
            self.session = session

            if !session.start() {
                self.session = nil
                continuation.resume(throwing: WebAuthenticationError.sessionStartFailed)
            }
        }
    }

    func cancel() {
        session?.cancel()
        session = nil
    }

    func presentationAnchor(for _: ASWebAuthenticationSession) -> ASPresentationAnchor {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let windows = scenes.flatMap(\.windows)
        guard let anchor = windows.first(where: \.isKeyWindow) ?? windows.first else {
            preconditionFailure("Xbox 登录需要可见窗口")
        }
        return anchor
    }
}
