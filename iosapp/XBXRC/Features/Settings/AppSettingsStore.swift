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

@MainActor
protocol CloudRegionSettingsProviding: AnyObject {
    var cloudRegionPreset: CloudRegionPreset { get }
    var usesEphemeralLoginSession: Bool { get }
}

@MainActor
final class AppSettingsStore: ObservableObject, CloudRegionSettingsProviding {
    static let cloudRegionKey = "ios.cloud.forceRegionPreset"
    static let ephemeralLoginKey = "ios.auth.usesEphemeralWebSession"
    static let appearanceModeKey = "ios.appearance.mode"
    static let appIconPresetKey = "ios.appearance.appIconPreset"

    @Published var appearanceMode: AppAppearanceMode {
        didSet {
            defaults.set(appearanceMode.rawValue, forKey: Self.appearanceModeKey)
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
            defaults.set(usesEphemeralLoginSession, forKey: Self.ephemeralLoginKey)
            IOSRuntimeTrace.state(
                domain: "settings",
                event: "ephemeralLoginSessionChanged",
                payload: ["enabled": .bool(usesEphemeralLoginSession)],
                dimension: .core,
                importance: .key
            )
        }
    }

    private let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        appearanceMode = defaults.string(forKey: Self.appearanceModeKey)
            .flatMap(AppAppearanceMode.init(rawValue:)) ?? .system
        appIconPreset = defaults.string(forKey: Self.appIconPresetKey)
            .flatMap(AppIconPreset.init(rawValue:)) ?? .default
        cloudRegionPreset = defaults.string(forKey: Self.cloudRegionKey)
            .flatMap(CloudRegionPreset.init(rawValue:)) ?? .default
        usesEphemeralLoginSession = defaults.bool(forKey: Self.ephemeralLoginKey)
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
            defaults.set(preset.rawValue, forKey: Self.appIconPresetKey)
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
        defaults.set(preset.rawValue, forKey: Self.cloudRegionKey)
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
}
