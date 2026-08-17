import Combine
import Foundation
import SwiftUI
import UIKit

enum AppAppearanceMode: String, CaseIterable, Identifiable, Sendable {
    case system
    case light
    case dark

    var id: String { rawValue }

    var title: String {
        switch self {
        case .system: "跟随系统"
        case .light: "亮色"
        case .dark: "暗色"
        }
    }

    var colorScheme: ColorScheme? {
        switch self {
        case .system: nil
        case .light: .light
        case .dark: .dark
        }
    }
}

enum AppIconPreset: String, CaseIterable, Identifiable, Sendable {
    case `default`
    case forest
    case midnight

    var id: String { rawValue }

    /// 传给 UIApplication 的资源名称；nil 表示恢复主图标。
    var alternateIconName: String? {
        switch self {
        case .default: nil
        case .forest: "AppIconForest"
        case .midnight: "AppIconMidnight"
        }
    }

    var title: String {
        switch self {
        case .default: "经典绿"
        case .forest: "森林绿"
        case .midnight: "午夜黑"
        }
    }

    var description: String {
        switch self {
        case .default: "XBXRC 默认图标"
        case .forest: "低饱和森林绿"
        case .midnight: "深色背景高对比度"
        }
    }

    var systemImage: String {
        switch self {
        case .default: "square.on.square"
        case .forest: "leaf"
        case .midnight: "moon.stars"
        }
    }
}

enum CloudRegionPreset: String, CaseIterable, Identifiable, Sendable {
    case `default`
    case australia
    case brazil
    case europe
    case japan
    case korea
    case unitedStates
    case southIndia
    case centralIndia

    var id: String { rawValue }

    var title: String {
        switch self {
        case .default: "默认"
        case .australia: "澳大利亚"
        case .brazil: "巴西"
        case .europe: "欧洲"
        case .japan: "日本"
        case .korea: "韩国"
        case .unitedStates: "美国"
        case .southIndia: "南印度"
        case .centralIndia: "中印度"
        }
    }

    var forceRegionIP: String {
        switch self {
        case .default: ""
        case .australia: "203.41.44.20"
        case .brazil: "200.221.11.101"
        case .europe: "194.25.0.68"
        case .japan: "210.131.113.123"
        case .korea: "168.126.63.1"
        case .unitedStates: "4.2.2.2"
        case .southIndia: "104.211.224.146"
        case .centralIndia: "104.211.96.159"
        }
    }
}

enum StreamingResolutionOption: Int, CaseIterable, Identifiable, Sendable {
    case p720 = 720
    case p1080 = 1080
    case p1080Hq = 1081
    case p1440 = 1440

    var id: Int { rawValue }

    var title: String {
        switch self {
        case .p720: "720p"
        case .p1080: "1080p"
        case .p1080Hq: "自动（1080p HQ）"
        case .p1440: "1440p"
        }
    }

    static let cloudChoices: [StreamingResolutionOption] = [.p1440, .p1080Hq, .p1080, .p720]
    static let homeChoices: [StreamingResolutionOption] = [.p1080Hq, .p1080, .p720]
}

enum StreamingBitrateMode: String, CaseIterable, Identifiable, Sendable {
    case auto = "Auto"
    case custom = "Custom"

    var id: String { rawValue }

    var title: String {
        switch self {
        case .auto: "自动"
        case .custom: "手动"
        }
    }
}

enum StreamingCodecOption: String, CaseIterable, Identifiable, Sendable {
    case auto = ""
    case h264High = "video/H264-64"
    case h264Main = "video/H264-4d"
    case h264Normal = "video/H264-42e"
    case h264Low = "video/H264-420"

    var id: String {
        rawValue.isEmpty ? "auto" : rawValue
    }

    var title: String {
        switch self {
        case .auto: "自动"
        case .h264High: "H.264 High"
        case .h264Main: "H.264 Main"
        case .h264Normal: "H.264 Constrained Baseline"
        case .h264Low: "H.264 Baseline"
        }
    }

    var summary: String {
        switch self {
        case .auto: "自动"
        case .h264High: "更清晰"
        case .h264Main: "更均衡"
        case .h264Normal: "更兼容"
        case .h264Low: "最兼容"
        }
    }
}

struct PreferredGameLanguageOption: Identifiable, Hashable, Sendable {
    let code: String
    let title: String

    var id: String { code }
}

enum PreferredGameLanguageCatalog {
    static let all: [PreferredGameLanguageOption] = [
        .init(code: "en-US", title: "English (United States)"),
        .init(code: "", title: "默认"),
        .init(code: "ar-SA", title: "Arabic (Saudi Arabia)"),
        .init(code: "cs-CZ", title: "Czech"),
        .init(code: "da-DK", title: "Danish"),
        .init(code: "de-DE", title: "German"),
        .init(code: "el-GR", title: "Greek"),
        .init(code: "en-GB", title: "English (United Kingdom)"),
        .init(code: "es-ES", title: "Spanish (Spain)"),
        .init(code: "es-MX", title: "Spanish (Mexico)"),
        .init(code: "fi-FI", title: "Finnish"),
        .init(code: "fr-FR", title: "French"),
        .init(code: "he-IL", title: "Hebrew"),
        .init(code: "hu-HU", title: "Hungarian"),
        .init(code: "it-IT", title: "Italian"),
        .init(code: "ja-JP", title: "日本語"),
        .init(code: "ko-KR", title: "Korean"),
        .init(code: "nb-NO", title: "Norwegian"),
        .init(code: "nl-NL", title: "Dutch"),
        .init(code: "pl-PL", title: "Polish"),
        .init(code: "pt-BR", title: "Portuguese (Brazil)"),
        .init(code: "pt-PT", title: "Portuguese (Portugal)"),
        .init(code: "ru-RU", title: "Russian"),
        .init(code: "sk-SK", title: "Slovak"),
        .init(code: "sv-SE", title: "Swedish"),
        .init(code: "tr-TR", title: "Turkish"),
        .init(code: "zh-CN", title: "简体中文"),
        .init(code: "zh-TW", title: "繁体中文"),
    ]

    static func title(for code: String) -> String {
        all.first(where: { $0.code == code })?.title ?? code
    }
}

struct StreamingSessionSettingsSnapshot: Equatable, Sendable {
    let preferredGameLocale: String
    let cloudResolution: Int
    let homeResolution: Int
    let preferIPv6: Bool
    let videoCodec: String
    let homeBitrateMode: String
    let homeBitrateMbps: Int
    let cloudBitrateMode: String
    let cloudBitrateMbps: Int
    let audioBitrateMode: String
    let audioBitrateKbps: Int
    let homeTurnFallback: Bool

    static let standard = StreamingSessionSettingsSnapshot(
        preferredGameLocale: "en-US",
        cloudResolution: StreamingResolutionOption.p720.rawValue,
        homeResolution: StreamingResolutionOption.p1080.rawValue,
        preferIPv6: false,
        videoCodec: StreamingCodecOption.auto.rawValue,
        homeBitrateMode: StreamingBitrateMode.auto.rawValue,
        homeBitrateMbps: 20,
        cloudBitrateMode: StreamingBitrateMode.auto.rawValue,
        cloudBitrateMbps: 20,
        audioBitrateMode: StreamingBitrateMode.auto.rawValue,
        audioBitrateKbps: 20,
        homeTurnFallback: true
    )
}

@MainActor
protocol CloudRegionSettingsProviding: AnyObject {
    var cloudRegionPreset: CloudRegionPreset { get }
    var usesEphemeralLoginSession: Bool { get }
}

@MainActor
protocol StreamingSessionSettingsProviding: AnyObject {
    var streamingSessionSettings: StreamingSessionSettingsSnapshot { get }
}

@MainActor
protocol PreferredGameLocaleProviding: AnyObject {
    var preferredGameLocale: String { get }
}

@MainActor
final class AppSettingsStore: ObservableObject, CloudRegionSettingsProviding, StreamingSessionSettingsProviding, PreferredGameLocaleProviding {
    static let cloudRegionKey = "ios.cloud.forceRegionPreset"
    static let ephemeralLoginKey = "ios.auth.usesEphemeralWebSession"
    static let appearanceModeKey = "ios.appearance.mode"
    static let appIconPresetKey = "ios.appearance.appIconPreset"
    static let preferredGameLocaleKey = "ios.streaming.preferredGameLocale"
    static let cloudResolutionKey = "ios.streaming.cloudResolution"
    static let homeResolutionKey = "ios.streaming.homeResolution"
    static let preferIPv6Key = "ios.streaming.preferIPv6"
    static let codecPreferenceKey = "ios.streaming.codecPreference"
    static let homeBitrateModeKey = "ios.streaming.homeBitrateMode"
    static let homeBitrateMbpsKey = "ios.streaming.homeBitrateMbps"
    static let cloudBitrateModeKey = "ios.streaming.cloudBitrateMode"
    static let cloudBitrateMbpsKey = "ios.streaming.cloudBitrateMbps"
    static let audioBitrateModeKey = "ios.streaming.audioBitrateMode"
    static let audioBitrateKbpsKey = "ios.streaming.audioBitrateKbps"
    static let homeTurnFallbackKey = "ios.streaming.homeTurnFallback"

    @Published var appearanceMode: AppAppearanceMode {
        didSet {
            guard appearanceMode != oldValue else { return }
            persistSettings()
            IOSRuntimeTrace.state(
                domain: "settings",
                event: "appearanceModeChanged",
                payload: ["mode": .string(appearanceMode.rawValue)],
                dimension: .core,
                importance: .key
            )
        }
    }

    @Published private(set) var appIconPreset: AppIconPreset

    @Published private(set) var cloudRegionPreset: CloudRegionPreset
    @Published var usesEphemeralLoginSession: Bool {
        didSet {
            guard usesEphemeralLoginSession != oldValue else { return }
            persistSettings()
            IOSRuntimeTrace.state(
                domain: "settings",
                event: "ephemeralLoginSessionChanged",
                payload: ["enabled": .bool(usesEphemeralLoginSession)],
                dimension: .core,
                importance: .key
            )
        }
    }

    @Published var preferredGameLocale: String {
        didSet {
            guard preferredGameLocale != oldValue else { return }
            persistSettings()
            traceStreamingSettingChange(
                key: "preferred_game_language",
                value: .string(preferredGameLocale.isEmpty ? "default" : preferredGameLocale)
            )
        }
    }

    @Published var cloudResolution: StreamingResolutionOption {
        didSet {
            guard cloudResolution != oldValue else { return }
            persistSettings()
            traceStreamingSettingChange(
                key: "resolution",
                value: .integer(Int64(cloudResolution.rawValue))
            )
        }
    }

    @Published var homeResolution: StreamingResolutionOption {
        didSet {
            guard homeResolution != oldValue else { return }
            persistSettings()
            traceStreamingSettingChange(
                key: "xhome_resolution",
                value: .integer(Int64(homeResolution.rawValue))
            )
        }
    }

    @Published var preferIPv6: Bool {
        didSet {
            guard preferIPv6 != oldValue else { return }
            persistSettings()
            traceStreamingSettingChange(key: "ipv6", value: .bool(preferIPv6))
        }
    }

    @Published var codecPreference: StreamingCodecOption {
        didSet {
            guard codecPreference != oldValue else { return }
            persistSettings()
            traceStreamingSettingChange(
                key: "codec",
                value: .string(codecPreference.rawValue.isEmpty ? "auto" : codecPreference.rawValue)
            )
        }
    }

    @Published var homeBitrateMode: StreamingBitrateMode {
        didSet {
            guard homeBitrateMode != oldValue else { return }
            persistSettings()
            traceStreamingSettingChange(
                key: "xhome_bitrate_mode",
                value: .string(homeBitrateMode.rawValue)
            )
        }
    }

    @Published var homeBitrateMbps: Int {
        didSet {
            guard homeBitrateMbps != oldValue else { return }
            persistSettings()
            traceStreamingSettingChange(
                key: "xhome_bitrate",
                value: .integer(Int64(homeBitrateMbps))
            )
        }
    }

    @Published var cloudBitrateMode: StreamingBitrateMode {
        didSet {
            guard cloudBitrateMode != oldValue else { return }
            persistSettings()
            traceStreamingSettingChange(
                key: "xcloud_bitrate_mode",
                value: .string(cloudBitrateMode.rawValue)
            )
        }
    }

    @Published var cloudBitrateMbps: Int {
        didSet {
            guard cloudBitrateMbps != oldValue else { return }
            persistSettings()
            traceStreamingSettingChange(
                key: "xcloud_bitrate",
                value: .integer(Int64(cloudBitrateMbps))
            )
        }
    }

    @Published var audioBitrateMode: StreamingBitrateMode {
        didSet {
            guard audioBitrateMode != oldValue else { return }
            persistSettings()
            traceStreamingSettingChange(
                key: "audio_bitrate_mode",
                value: .string(audioBitrateMode.rawValue)
            )
        }
    }

    @Published var audioBitrateKbps: Int {
        didSet {
            guard audioBitrateKbps != oldValue else { return }
            persistSettings()
            traceStreamingSettingChange(
                key: "audio_bitrate",
                value: .integer(Int64(audioBitrateKbps))
            )
        }
    }

    @Published var homeTurnFallbackEnabled: Bool {
        didSet {
            guard homeTurnFallbackEnabled != oldValue else { return }
            persistSettings()
            traceStreamingSettingChange(
                key: "xhome_turn_fallback",
                value: .bool(homeTurnFallbackEnabled)
            )
        }
    }

    private let persistence: AppSettingsPersistence

    init(defaults: UserDefaults = .standard) {
        let persistence = AppSettingsPersistence(defaults: defaults)
        let snapshot = persistence.load()
        self.persistence = persistence
        appearanceMode = AppAppearanceMode(rawValue: snapshot.appearanceMode) ?? .system
        appIconPreset = AppIconPreset(rawValue: snapshot.appIconPreset) ?? .default
        cloudRegionPreset = CloudRegionPreset(rawValue: snapshot.cloudRegionPreset) ?? .default
        usesEphemeralLoginSession = snapshot.usesEphemeralLoginSession
        preferredGameLocale = snapshot.preferredGameLocale
        cloudResolution = StreamingResolutionOption(rawValue: snapshot.cloudResolution) ?? .p720
        homeResolution = StreamingResolutionOption(rawValue: snapshot.homeResolution) ?? .p1080
        preferIPv6 = snapshot.preferIPv6
        codecPreference = StreamingCodecOption(rawValue: snapshot.codecPreference) ?? .auto
        homeBitrateMode = StreamingBitrateMode(rawValue: snapshot.homeBitrateMode) ?? .auto
        homeBitrateMbps = snapshot.homeBitrateMbps
        cloudBitrateMode = StreamingBitrateMode(rawValue: snapshot.cloudBitrateMode) ?? .auto
        cloudBitrateMbps = snapshot.cloudBitrateMbps
        audioBitrateMode = StreamingBitrateMode(rawValue: snapshot.audioBitrateMode) ?? .auto
        audioBitrateKbps = snapshot.audioBitrateKbps
        homeTurnFallbackEnabled = snapshot.homeTurnFallbackEnabled
        persistence.save(snapshot)
    }

    /// 仅在系统确认切换成功后提交偏好，失败时保留当前图标。
    @discardableResult
    func setAppIconPreset(_ preset: AppIconPreset) async -> String? {
        guard preset != appIconPreset else { return nil }
        guard UIApplication.shared.supportsAlternateIcons else {
            return "当前设备不支持切换应用图标"
        }

        let error = await withCheckedContinuation { continuation in
            UIApplication.shared.setAlternateIconName(preset.alternateIconName) { error in
                continuation.resume(returning: error)
            }
        }
        guard let error else {
            appIconPreset = preset
            persistSettings()
            IOSRuntimeTrace.state(
                domain: "settings",
                event: "appIconPresetChanged",
                payload: ["preset": .string(preset.rawValue)],
                dimension: .core,
                importance: .key
            )
            return nil
        }
        return "应用图标切换失败：\(error.localizedDescription)"
    }

    @discardableResult
    func setCloudRegionPreset(_ preset: CloudRegionPreset) -> Bool {
        guard preset != cloudRegionPreset else { return false }
        cloudRegionPreset = preset
        persistSettings()
        IOSRuntimeTrace.state(
            domain: "settings",
            event: "cloudRegionPresetChanged",
            payload: [
                "preset": .string(preset.rawValue),
                "forceRegionApplied": .bool(!preset.forceRegionIP.isEmpty),
            ],
            dimension: .core,
            importance: .key
        )
        return true
    }

    var preferredGameLanguageTitle: String {
        PreferredGameLanguageCatalog.title(for: preferredGameLocale)
    }

    var streamingSessionSettings: StreamingSessionSettingsSnapshot {
        StreamingSessionSettingsSnapshot(
            preferredGameLocale: PreferredGameLocaleNormalizer.normalize(preferredGameLocale),
            cloudResolution: cloudResolution.rawValue,
            homeResolution: homeResolution.rawValue,
            preferIPv6: preferIPv6,
            videoCodec: codecPreference.rawValue,
            homeBitrateMode: homeBitrateMode.rawValue,
            homeBitrateMbps: Self.clamp(homeBitrateMbps, min: 1, max: 200),
            cloudBitrateMode: cloudBitrateMode.rawValue,
            cloudBitrateMbps: Self.clamp(cloudBitrateMbps, min: 1, max: 200),
            audioBitrateMode: audioBitrateMode.rawValue,
            audioBitrateKbps: Self.clamp(audioBitrateKbps, min: 1, max: 512),
            homeTurnFallback: homeTurnFallbackEnabled
        )
    }

    private func persistSettings() {
        persistence.save(
            AppSettingsSnapshotV1(
                schemaVersion: AppSettingsSnapshotV1.schemaVersion,
                appearanceMode: appearanceMode.rawValue,
                appIconPreset: appIconPreset.rawValue,
                cloudRegionPreset: cloudRegionPreset.rawValue,
                usesEphemeralLoginSession: usesEphemeralLoginSession,
                preferredGameLocale: PreferredGameLocaleNormalizer.normalizePreference(preferredGameLocale),
                cloudResolution: cloudResolution.rawValue,
                homeResolution: homeResolution.rawValue,
                preferIPv6: preferIPv6,
                codecPreference: codecPreference.rawValue,
                homeBitrateMode: homeBitrateMode.rawValue,
                homeBitrateMbps: Self.clamp(homeBitrateMbps, min: 1, max: 200),
                cloudBitrateMode: cloudBitrateMode.rawValue,
                cloudBitrateMbps: Self.clamp(cloudBitrateMbps, min: 1, max: 200),
                audioBitrateMode: audioBitrateMode.rawValue,
                audioBitrateKbps: Self.clamp(audioBitrateKbps, min: 1, max: 512),
                homeTurnFallbackEnabled: homeTurnFallbackEnabled
            )
        )
    }

    private func traceStreamingSettingChange(
        key: String,
        value: IOSRuntimeTraceValue
    ) {
        IOSRuntimeTrace.state(
            domain: "settings",
            event: "streamingSettingChanged",
            payload: [
                "key": .string(key),
                "value": value,
            ],
            dimension: .core,
            importance: .key
        )
    }

    private static func clamp(_ value: Int, min minValue: Int, max maxValue: Int) -> Int {
        Swift.max(minValue, Swift.min(maxValue, value))
    }
}
