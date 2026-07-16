import Foundation
import Security

struct StoredAuthSession: Codable, Equatable, Sendable {
    let refreshToken: String
    let seedJSON: String
    let webTokenJSON: String
    let appLevel: UInt32
    let cloudAccountID: String?
    let cloudRegionHost: String?

    init(
        refreshToken: String,
        seedJSON: String,
        webTokenJSON: String,
        appLevel: UInt32,
        cloudAccountID: String? = nil,
        cloudRegionHost: String? = nil
    ) {
        self.refreshToken = refreshToken
        self.seedJSON = seedJSON
        self.webTokenJSON = webTokenJSON
        self.appLevel = appLevel
        self.cloudAccountID = cloudAccountID
        self.cloudRegionHost = cloudRegionHost
    }

    init(
        bridgeSession: AuthSession,
        cloudAccountID: String? = nil,
        cloudRegionHost: String? = nil
    ) {
        refreshToken = bridgeSession.refreshToken
        seedJSON = bridgeSession.seedJson
        webTokenJSON = bridgeSession.webTokenJson
        appLevel = bridgeSession.appLevel
        self.cloudAccountID = cloudAccountID
        self.cloudRegionHost = cloudRegionHost
    }
}

protocol AuthSessionStoring: Sendable {
    func load() async throws -> StoredAuthSession?
    func save(_ session: StoredAuthSession) async throws
    func delete() async throws
}

enum KeychainSessionError: LocalizedError {
    case unexpectedData
    case operationFailed(OSStatus)

    var errorDescription: String? {
        switch self {
        case .unexpectedData:
            "登录凭据格式无效"
        case let .operationFailed(status):
            SecCopyErrorMessageString(status, nil) as String? ?? "Keychain 操作失败"
        }
    }
}

actor KeychainSessionStore: AuthSessionStoring {
    private let service = "com.xuasir.xbxrc.ios.auth"
    private let account = "xbox-session"

    func load() async throws -> StoredAuthSession? {
        var query = baseQuery
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne

        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound {
            return nil
        }
        guard status == errSecSuccess else {
            throw KeychainSessionError.operationFailed(status)
        }
        guard let data = result as? Data else {
            throw KeychainSessionError.unexpectedData
        }
        return try JSONDecoder().decode(StoredAuthSession.self, from: data)
    }

    func save(_ session: StoredAuthSession) async throws {
        let data = try JSONEncoder().encode(session)
        let attributes: [String: Any] = [
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
        ]
        let updateStatus = SecItemUpdate(baseQuery as CFDictionary, attributes as CFDictionary)
        if updateStatus == errSecSuccess {
            return
        }
        guard updateStatus == errSecItemNotFound else {
            throw KeychainSessionError.operationFailed(updateStatus)
        }

        var item = baseQuery
        attributes.forEach { item[$0.key] = $0.value }
        let addStatus = SecItemAdd(item as CFDictionary, nil)
        guard addStatus == errSecSuccess else {
            throw KeychainSessionError.operationFailed(addStatus)
        }
    }

    func delete() async throws {
        let status = SecItemDelete(baseQuery as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw KeychainSessionError.operationFailed(status)
        }
    }

    private var baseQuery: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrSynchronizable as String: false,
        ]
    }
}
