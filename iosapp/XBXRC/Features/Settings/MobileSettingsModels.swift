import Foundation
import Security

enum MobileVideoFormat: String, CaseIterable, Identifiable, Sendable {
    case fit, fill, stretch, ratio16x9, ratio16x10, ratio18x9, ratio21x9, ratio4x3
    var id: String { rawValue }
    var title: String {
        switch self {
        case .fit: "完整显示"
        case .fill: "填满屏幕"
        case .stretch: "拉伸至全屏"
        case .ratio16x9: "16:9"
        case .ratio16x10: "16:10"
        case .ratio18x9: "18:9"
        case .ratio21x9: "21:9"
        case .ratio4x3: "4:3"
        }
    }
}

enum MobileMenuShortcut: String, Codable, CaseIterable, Identifiable, Sendable {
    case menuView, bumpers, stickButtons
    var id: String { rawValue }
    var mask: UInt16 {
        switch self {
        case .menuView: 4 | 8
        case .bumpers: 4096 | 8192
        case .stickButtons: 16384 | 32768
        }
    }
    var title: String {
        switch self {
        case .menuView: "视图 + 菜单"
        case .bumpers: "LB + RB"
        case .stickButtons: "L3 + R3"
        }
    }
    var buttons: [MobileShortcutButton] {
        switch self {
        case .menuView: [.view, .menu]
        case .bumpers: [.leftShoulder, .rightShoulder]
        case .stickButtons: [.leftStick, .rightStick]
        }
    }
}

enum MobileShortcutTarget: String, CaseIterable, Identifiable, Sendable {
    case menu, diagnostics
    var id: String { rawValue }
    var title: String { self == .menu ? "串流菜单" : "诊断菜单" }
}

enum MobileVibrationStrength: String, Codable, CaseIterable, Identifiable, Sendable {
    case realistic, enhanced, full
    var id: String { rawValue }
    var title: String {
        switch self {
        case .realistic: "真实"
        case .enhanced: "增强"
        case .full: "全强"
        }
    }
    // 与 PC rumble.ts 共用数值，先按马达曲线调节，再进入设备通道路由。
    func magnitude(_ value: Float, trigger: Bool) -> Float {
        let scale: Double
        let gamma: Double
        switch self {
        case .realistic:
            scale = trigger ? 0.58 : 0.66
            gamma = trigger ? 1.26 : 1.32
        case .enhanced:
            scale = trigger ? 0.74 : 0.82
            gamma = trigger ? 1.14 : 1.18
        case .full:
            scale = 1
            gamma = 1
        }
        return value.isFinite ? Float(pow(Double(min(1, max(0, value))), gamma) * scale) : 0
    }
}

struct MobileVibrationChannels: Codable, Equatable, Sendable {
    var leftHandle: MobileVibrationStrength = .realistic
    var rightHandle: MobileVibrationStrength = .realistic
    var leftTrigger: MobileVibrationStrength = .realistic
    var rightTrigger: MobileVibrationStrength = .realistic

    init(_ strength: MobileVibrationStrength = .realistic) {
        leftHandle = strength
        rightHandle = strength
        leftTrigger = strength
        rightTrigger = strength
    }

    var uniformStrength: MobileVibrationStrength? {
        leftHandle == rightHandle && leftHandle == leftTrigger && leftHandle == rightTrigger
            ? leftHandle : nil
    }
}

enum MobileShortcutButton: String, Codable, CaseIterable, Identifiable, Sendable {
    case a, b, x, y, leftShoulder, rightShoulder, leftTrigger, rightTrigger
    case view, menu, leftStick, rightStick, up, down, left, right, home
    var id: String { rawValue }
    var title: String {
        switch self {
        case .a: "A"
        case .b: "B"
        case .x: "X"
        case .y: "Y"
        case .leftShoulder: "LB"
        case .rightShoulder: "RB"
        case .leftTrigger: "LT"
        case .rightTrigger: "RT"
        case .view: "视图"
        case .menu: "菜单"
        case .leftStick: "L3"
        case .rightStick: "R3"
        case .up: "方向上"
        case .down: "方向下"
        case .left: "方向左"
        case .right: "方向右"
        case .home: "Xbox"
        }
    }
    private var mask: UInt16 {
        switch self {
        case .home: 2
        case .menu: 4
        case .view: 8
        case .a: 16
        case .b: 32
        case .x: 64
        case .y: 128
        case .up: 256
        case .down: 512
        case .left: 1024
        case .right: 2048
        case .leftShoulder: 4096
        case .rightShoulder: 8192
        case .leftStick: 16384
        case .rightStick: 32768
        case .leftTrigger, .rightTrigger: 0
        }
    }
    func isPressed(in state: IOSStreamGamepadState) -> Bool {
        switch self {
        case .leftTrigger: state.leftTrigger > 0.5
        case .rightTrigger: state.rightTrigger > 0.5
        default: state.buttonMask & mask != 0
        }
    }
    func value(in state: IOSStreamGamepadState) -> Float {
        switch self {
        case .leftTrigger: state.leftTrigger
        case .rightTrigger: state.rightTrigger
        default: state.buttonMask & mask != 0 ? 1 : 0
        }
    }
    func write(_ value: Float, to state: inout IOSStreamGamepadState) {
        let safe = value.isFinite ? min(1, max(0, value)) : 0
        switch self {
        case .leftTrigger: state.leftTrigger = safe
        case .rightTrigger: state.rightTrigger = safe
        default:
            if safe > 0.5 { state.buttonMask |= mask } else { state.buttonMask &= ~mask }
        }
    }
    static func normalized(_ buttons: [Self]) -> [Self] {
        Array(allCases.filter { buttons.contains($0) }.prefix(4))
    }
}

/// 单次长按消费完整组合；全部松开后重新待命，重叠组合按按键数量优先。
struct MobileShortcutMatcher {
    private var target: MobileShortcutTarget?
    private var buttons: [MobileShortcutButton] = []
    private var startedAt = 0.0
    private var consumed = false

    mutating func reset() { self = Self() }

    mutating func sample(
        _ state: IOSStreamGamepadState, preferences: MobilePreferences, now: Double
    )
        -> (captured: Bool, action: MobileShortcutTarget?)
    {
        if consumed {
            if buttons.contains(where: { $0.isPressed(in: state) }) { return (true, nil) }
            reset()
        }
        let matches = MobileShortcutTarget.allCases.filter {
            let binding = preferences.shortcutButtons(for: $0)
            return !binding.isEmpty && binding.allSatisfy { $0.isPressed(in: state) }
        }.sorted {
            let first = preferences.shortcutButtons(for: $0).count
            let second = preferences.shortcutButtons(for: $1).count
            return first == second ? $0 == .menu : first > second
        }
        guard let match = matches.first else {
            reset()
            return (false, nil)
        }
        let binding = preferences.shortcutButtons(for: match)
        if target != match || buttons != binding {
            target = match
            buttons = binding
            startedAt = now
        }
        if now - startedAt >= preferences.menuHoldDuration * 1_000 {
            consumed = true
            return (true, match)
        }
        return (true, nil)
    }
}

enum MobileColorPreset: String, CaseIterable, Identifiable {
    case standard, clear, soft
    var id: String { rawValue }
    var title: String {
        switch self {
        case .standard: "标准"
        case .clear: "清晰"
        case .soft: "柔和"
        }
    }
    var values: (Double, Double, Double) {
        switch self {
        case .standard: (100, 100, 100)
        case .clear: (105, 105, 100)
        case .soft: (96, 96, 102)
        }
    }
}

struct MobilePicturePreferences: Codable, Equatable, Sendable {
    var compatibilityMode = false
    var superResolution = false
    var sharpness = 0
    var saturation = 100.0
    var contrast = 100.0
    var brightness = 100.0

    var colorPreset: MobileColorPreset? {
        MobileColorPreset.allCases.first {
            let v = $0.values
            return saturation == v.0 && contrast == v.1 && brightness == v.2
        }
    }

    mutating func apply(_ preset: MobileColorPreset) {
        (saturation, contrast, brightness) = preset.values
    }

    func normalized() -> Self {
        var value = self
        value.sharpness = sharpness <= 0 ? 0 : sharpness <= 2 ? 1 : 3
        value.saturation = Self.color(saturation)
        value.contrast = Self.color(contrast)
        value.brightness = Self.color(brightness)
        return value
    }

    private static func color(_ value: Double) -> Double {
        value.isFinite ? min(200, max(0, value.rounded())) : 100
    }
}

struct MobileTurnConfiguration: Codable, Equatable, Sendable {
    let serverURL: String
    let username: String
    let credential: String

    enum CodingKeys: String, CodingKey {
        case serverURL = "server_url"
        case username = "server_username"
        case credential = "server_credential"
    }

    static func imported(from data: Data) throws -> Self {
        guard data.count <= 16_384 else { throw MobileTurnImportError.invalid }
        let raw = try JSONDecoder().decode(Self.self, from: data)
        let url = raw.serverURL.trimmingCharacters(in: .whitespacesAndNewlines)
        let username = raw.username.trimmingCharacters(in: .whitespacesAndNewlines)
        let credential = raw.credential.trimmingCharacters(in: .whitespacesAndNewlines)
        guard url.utf8.count <= 2_048, username.utf8.count <= 512, credential.utf8.count <= 4_096,
            !username.isEmpty, !credential.isEmpty,
            let components = URLComponents(string: url),
            ["turn", "turns"].contains(components.scheme?.lowercased() ?? ""),
            !url.contains(where: { $0.isWhitespace }),
            !url.contains("@"), !url.contains("#")
        else { throw MobileTurnImportError.invalid }
        // TURN URI 使用不含 // 的 RFC 7065 格式；按 authority 校验地址和端口。
        guard let separator = url.firstIndex(of: ":") else { throw MobileTurnImportError.invalid }
        let authority = String(url[url.index(after: separator)...])
        guard !authority.isEmpty, !authority.hasPrefix("//"),
            let endpoint = URLComponents(string: "https://" + authority),
            let host = endpoint.host, !host.isEmpty,
            endpoint.path.isEmpty,
            endpoint.port.map({ (1...65_535).contains($0) }) ?? true,
            endpoint.queryItems?.allSatisfy({
                $0.name == "transport" && ["udp", "tcp"].contains($0.value ?? "")
            }) ?? true
        else { throw MobileTurnImportError.invalid }
        return Self(
            serverURL: (components.scheme?.lowercased() ?? "turn") + ":" + authority,
            username: username, credential: credential)
    }
}

enum MobileTurnImportError: LocalizedError {
    case invalid
    var errorDescription: String? { "请选择有效的 TURN JSON 配置，包含服务器、用户名和凭据。" }
}

/// TURN 私有凭据与 Xbox 登录分别保存在本机 Keychain。
enum MobileTurnKeychain {
    private static var query: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "com.xuasir.xbxrc.ios.turn",
            kSecAttrAccount as String: "custom-relay",
            kSecAttrSynchronizable as String: false,
        ]
    }
    static func load() -> MobileTurnConfiguration? {
        var request = query
        request[kSecReturnData as String] = true
        request[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(request as CFDictionary, &result) == errSecSuccess,
            let data = result as? Data
        else { return nil }
        return try? MobileTurnConfiguration.imported(from: data)
    }
    static func save(_ value: MobileTurnConfiguration) throws {
        let attributes: [String: Any] = [
            kSecValueData as String: try JSONEncoder().encode(value),
            kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
        ]
        let status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        if status == errSecSuccess { return }
        guard status == errSecItemNotFound else {
            throw KeychainSessionError.operationFailed(status)
        }
        var request = query
        for (key, value) in attributes { request[key] = value }
        let added = SecItemAdd(request as CFDictionary, nil)
        guard added == errSecSuccess else { throw KeychainSessionError.operationFailed(added) }
    }
    static func clear() throws {
        let status = SecItemDelete(query as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw KeychainSessionError.operationFailed(status)
        }
    }
}

enum MobileAppLanguage: String, Codable, CaseIterable, Identifiable, Sendable {
    case system
    case chinese = "zh-Hans"
    case english = "en"
    var id: String { rawValue }
    var title: String {
        switch self {
        case .system: "跟随系统"
        case .chinese: "简体中文"
        case .english: "English"
        }
    }
    var locale: Locale { self == .system ? .current : Locale(identifier: rawValue) }
}
