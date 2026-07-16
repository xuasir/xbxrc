import Combine
import Foundation

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
        cloudRegionPreset = defaults.string(forKey: Self.cloudRegionKey)
            .flatMap(CloudRegionPreset.init(rawValue:)) ?? .default
        usesEphemeralLoginSession = defaults.bool(forKey: Self.ephemeralLoginKey)
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
