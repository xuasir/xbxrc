import Foundation

enum TouchControllerMode: String, Codable, CaseIterable, Identifiable, Sendable {
    case automatic, always, hidden
    var id: String { rawValue }
    var title: String {
        switch self {
        case .automatic: "自动"
        case .always: "始终显示"
        case .hidden: "隐藏"
        }
    }
}

/// v2 记录移动平台偏好；既有 v1 连接设置按原合同迁移。
struct MobilePreferences: Codable, Equatable, Sendable {
    var schemaVersion = 2
    var catalogMarket = "US"
    var catalogLanguage = "en-US"
    var cloudServerRegion = "auto"
    var videoFit = "fit"
    var startMuted = false
    var powerOn = false
    var preventResolutionDrops = false
    var keepScreenAwake = true
    var touchControllerMode: TouchControllerMode = .automatic
    var touchOpacity = 0.65
    var leftStickDeadzone = 0.10
    var rightStickDeadzone = 0.10
    var invertLeftY = false
    var invertRightY = false
    var vibrationEnabled = true
    var vibrationIntensity = 1.0
    var vibrationChannels = MobileVibrationChannels()

    func vibrationEffect(_ effect: IOSStreamRumbleEffect) -> IOSStreamRumbleEffect {
        IOSStreamRumbleEffect(
            gamepadIndex: effect.gamepadIndex,
            strongMagnitude: vibrationChannels.leftHandle.magnitude(
                effect.strongMagnitude, trigger: false),
            weakMagnitude: vibrationChannels.rightHandle.magnitude(
                effect.weakMagnitude, trigger: false),
            leftTrigger: vibrationChannels.leftTrigger.magnitude(effect.leftTrigger, trigger: true),
            rightTrigger: vibrationChannels.rightTrigger.magnitude(
                effect.rightTrigger, trigger: true),
            durationMilliseconds: effect.durationMilliseconds, format: effect.format,
            delayMilliseconds: effect.delayMilliseconds, repeatCount: effect.repeatCount)
    }
    var menuHoldDuration = 0.6
    var appLanguage: MobileAppLanguage = .system
    var uiHaptics = false
    var uiAudio = true
    var enableAudioControl = false
    var compactPerformanceOverlay = false
    var gamepadBindings: [String: [String: MobileControlBinding]] = [:]
    var touchLayoutID = "classic"
    var touchScale = 1.0
    var touchLayoutTemplates: [TouchLayoutTemplate] = []
    var picture = MobilePicturePreferences()
    var menuShortcut: MobileMenuShortcut = .menuView
    var customMenuButtons: [MobileShortcutButton]?
    var diagnosticsButtons: [MobileShortcutButton] = []

    func shortcutButtons(for target: MobileShortcutTarget) -> [MobileShortcutButton] {
        target == .menu ? (customMenuButtons ?? menuShortcut.buttons) : diagnosticsButtons
    }

    func shortcutConflicts(_ buttons: [MobileShortcutButton], for target: MobileShortcutTarget)
        -> Bool
    {
        let binding = MobileShortcutButton.normalized(buttons)
        let other = MobileShortcutButton.normalized(
            shortcutButtons(for: target == .menu ? .diagnostics : .menu))
        return !binding.isEmpty && binding == other
    }

    mutating func setShortcut(_ buttons: [MobileShortcutButton], for target: MobileShortcutTarget) {
        if target == .menu {
            customMenuButtons = MobileShortcutButton.normalized(buttons)
        } else {
            diagnosticsButtons = MobileShortcutButton.normalized(buttons)
        }
    }

    static let persistenceKey = "ios.settings.mobile.v2"
    static let markets = ["US", "GB", "JP", "KR", "AU", "BR", "DE", "FR", "IN", "HK", "TW"]

    func normalized() -> Self {
        var value = self
        value.schemaVersion = 2
        value.catalogMarket =
            Self.markets.contains(catalogMarket.uppercased()) ? catalogMarket.uppercased() : "US"
        value.catalogLanguage = PreferredGameLocaleNormalizer.normalize(catalogLanguage)
        value.cloudServerRegion = cloudServerRegion.trimmingCharacters(in: .whitespacesAndNewlines)
        if value.cloudServerRegion.isEmpty { value.cloudServerRegion = "auto" }
        value.videoFit = MobileVideoFormat(rawValue: videoFit)?.rawValue ?? "fit"
        value.picture = picture.normalized()
        value.gamepadBindings = MobileControlBinding.normalized(gamepadBindings)
        value.touchLayoutTemplates = TouchLayoutTemplate.normalized(touchLayoutTemplates)
        value.touchLayoutID = UUID(uuidString: touchLayoutID)?.uuidString ?? touchLayoutID
        // 旧测试目录身份继续回到标准布局，本地模板按有效身份恢复。
        if !value.touchLayoutTemplates.contains(where: { $0.id == value.touchLayoutID }) {
            value.touchLayoutID = TouchLayoutTemplate.standardID
        }
        value.touchOpacity = clamp(touchOpacity, 0.2...1, fallback: 0.65)
        value.touchScale = clamp(touchScale, 0.75...1.5, fallback: 1)
        value.leftStickDeadzone = clamp(leftStickDeadzone, 0...0.3, fallback: 0.1)
        value.rightStickDeadzone = clamp(rightStickDeadzone, 0...0.3, fallback: 0.1)
        value.vibrationIntensity = clamp(vibrationIntensity, 0...1, fallback: 1)
        // 菜单沿统一触发阈值，旧版自定义时间恢复默认。
        value.menuHoldDuration = 0.6
        value.customMenuButtons = customMenuButtons.map(MobileShortcutButton.normalized)
        value.diagnosticsButtons = MobileShortcutButton.normalized(diagnosticsButtons)
        if value.shortcutConflicts(value.diagnosticsButtons, for: .diagnostics) {
            value.diagnosticsButtons = []
        }
        return value
    }

    private func clamp(_ value: Double, _ range: ClosedRange<Double>, fallback: Double) -> Double {
        value.isFinite ? min(range.upperBound, max(range.lowerBound, value)) : fallback
    }
}

// 增加字段时逐项使用默认值，保留已安装版本的 v2 用户设置。
extension MobilePreferences {
    init(from decoder: any Decoder) throws {
        self.init()
        let c = try decoder.container(keyedBy: CodingKeys.self)
        func value<T: Decodable>(_ key: CodingKeys, _ fallback: T) -> T {
            (try? c.decode(T.self, forKey: key)) ?? fallback
        }
        schemaVersion = value(.schemaVersion, schemaVersion)
        catalogMarket = value(.catalogMarket, catalogMarket)
        catalogLanguage = value(.catalogLanguage, catalogLanguage)
        cloudServerRegion = value(.cloudServerRegion, cloudServerRegion)
        videoFit = value(.videoFit, videoFit)
        startMuted = value(.startMuted, startMuted)
        powerOn = value(.powerOn, powerOn)
        preventResolutionDrops = value(.preventResolutionDrops, preventResolutionDrops)
        keepScreenAwake = value(.keepScreenAwake, keepScreenAwake)
        touchControllerMode = value(.touchControllerMode, touchControllerMode)
        touchOpacity = value(.touchOpacity, touchOpacity)
        leftStickDeadzone = value(.leftStickDeadzone, leftStickDeadzone)
        rightStickDeadzone = value(.rightStickDeadzone, rightStickDeadzone)
        invertLeftY = value(.invertLeftY, invertLeftY)
        invertRightY = value(.invertRightY, invertRightY)
        vibrationEnabled = value(.vibrationEnabled, vibrationEnabled)
        vibrationIntensity = value(.vibrationIntensity, vibrationIntensity)
        // 旧 v2 使用线性强度，保留其手感；新安装采用 PC 的真实预设。
        vibrationChannels = value(.vibrationChannels, MobileVibrationChannels(.full))
        menuHoldDuration = value(.menuHoldDuration, menuHoldDuration)
        appLanguage = value(.appLanguage, appLanguage)
        uiHaptics = value(.uiHaptics, uiHaptics)
        uiAudio = value(.uiAudio, uiAudio)
        enableAudioControl = value(.enableAudioControl, enableAudioControl)
        picture = value(.picture, picture)
        compactPerformanceOverlay = value(.compactPerformanceOverlay, compactPerformanceOverlay)
        gamepadBindings = value(.gamepadBindings, gamepadBindings)
        touchLayoutID = value(.touchLayoutID, touchLayoutID)
        touchScale = value(.touchScale, touchScale)
        touchLayoutTemplates =
            (try? c.decode(
                TouchLayoutTemplateList.self,
                forKey: .touchLayoutTemplates))?.templates ?? []
        menuShortcut = value(.menuShortcut, menuShortcut)
        customMenuButtons = value(.customMenuButtons, customMenuButtons)
        diagnosticsButtons = value(.diagnosticsButtons, diagnosticsButtons)
    }
}
