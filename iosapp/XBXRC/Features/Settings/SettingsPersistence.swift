import Foundation

enum PreferredGameLocaleNormalizer {
    static let fallback = "en-US"

    static func normalize(_ value: String) -> String {
        let preference = normalizePreference(value)
        return preference.isEmpty ? fallback : preference
    }

    static func normalizePreference(_ value: String) -> String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "" }
        if let known = PreferredGameLanguageCatalog.all.first(where: {
            !$0.code.isEmpty && $0.code.caseInsensitiveCompare(trimmed) == .orderedSame
        }) {
            return known.code
        }

        let components = trimmed.replacingOccurrences(of: "_", with: "-")
            .split(separator: "-", omittingEmptySubsequences: true)
        guard let language = components.first, language.count >= 2 else { return fallback }
        return components.enumerated().map { index, component in
            if index == 0 { return component.lowercased() }
            if component.count == 4 {
                return component.prefix(1).uppercased() + component.dropFirst().lowercased()
            }
            if component.count == 2 || component.count == 3 { return component.uppercased() }
            return component.lowercased()
        }.joined(separator: "-")
    }
}

struct AppSettingsSnapshotV1: Codable, Equatable {
    static let schemaVersion = 1

    let schemaVersion: Int
    let appearanceMode: String
    let appIconPreset: String
    let cloudRegionPreset: String
    let usesEphemeralLoginSession: Bool
    let preferredGameLocale: String
    let cloudResolution: Int
    let homeResolution: Int
    let preferIPv6: Bool
    let codecPreference: String
    let homeBitrateMode: String
    let homeBitrateMbps: Int
    let cloudBitrateMode: String
    let cloudBitrateMbps: Int
    let audioBitrateMode: String
    let audioBitrateKbps: Int
    let homeTurnFallbackEnabled: Bool
}

@MainActor
struct AppSettingsPersistence {
    static let snapshotKey = "ios.settings.snapshot.v1"

    let defaults: UserDefaults

    func load() -> AppSettingsSnapshotV1 {
        let cached = defaults.data(forKey: Self.snapshotKey).flatMap {
            try? JSONDecoder().decode(AppSettingsSnapshotV1.self, from: $0)
        }.flatMap { $0.schemaVersion == AppSettingsSnapshotV1.schemaVersion ? $0 : nil }

        return AppSettingsSnapshotV1(
            schemaVersion: AppSettingsSnapshotV1.schemaVersion,
            appearanceMode: enumRawValue(
                key: AppSettingsStore.appearanceModeKey,
                cached: cached?.appearanceMode,
                fallback: AppAppearanceMode.system.rawValue,
                parse: AppAppearanceMode.init(rawValue:)
            ),
            appIconPreset: enumRawValue(
                key: AppSettingsStore.appIconPresetKey,
                cached: cached?.appIconPreset,
                fallback: AppIconPreset.default.rawValue,
                parse: AppIconPreset.init(rawValue:)
            ),
            cloudRegionPreset: enumRawValue(
                key: AppSettingsStore.cloudRegionKey,
                cached: cached?.cloudRegionPreset,
                fallback: CloudRegionPreset.default.rawValue,
                parse: CloudRegionPreset.init(rawValue:)
            ),
            usesEphemeralLoginSession: boolValue(
                key: AppSettingsStore.ephemeralLoginKey,
                cached: cached?.usesEphemeralLoginSession,
                fallback: false
            ),
            preferredGameLocale: PreferredGameLocaleNormalizer.normalizePreference(
                stringValue(
                    key: AppSettingsStore.preferredGameLocaleKey,
                    cached: cached?.preferredGameLocale,
                    fallback: PreferredGameLocaleNormalizer.fallback
                )
            ),
            cloudResolution: resolutionValue(
                key: AppSettingsStore.cloudResolutionKey,
                cached: cached?.cloudResolution,
                fallback: StreamingResolutionOption.p720.rawValue,
                allowed: Set(StreamingResolutionOption.cloudChoices.map(\.rawValue))
            ),
            homeResolution: resolutionValue(
                key: AppSettingsStore.homeResolutionKey,
                cached: cached?.homeResolution,
                fallback: StreamingResolutionOption.p1080.rawValue,
                allowed: Set(StreamingResolutionOption.homeChoices.map(\.rawValue))
            ),
            preferIPv6: boolValue(
                key: AppSettingsStore.preferIPv6Key,
                cached: cached?.preferIPv6,
                fallback: false
            ),
            codecPreference: enumRawValue(
                key: AppSettingsStore.codecPreferenceKey,
                cached: cached?.codecPreference,
                fallback: StreamingCodecOption.auto.rawValue,
                parse: StreamingCodecOption.init(rawValue:)
            ),
            homeBitrateMode: enumRawValue(
                key: AppSettingsStore.homeBitrateModeKey,
                cached: cached?.homeBitrateMode,
                fallback: StreamingBitrateMode.auto.rawValue,
                parse: StreamingBitrateMode.init(rawValue:)
            ),
            homeBitrateMbps: integerValue(
                key: AppSettingsStore.homeBitrateMbpsKey,
                cached: cached?.homeBitrateMbps,
                fallback: 20,
                range: 1 ... 200
            ),
            cloudBitrateMode: enumRawValue(
                key: AppSettingsStore.cloudBitrateModeKey,
                cached: cached?.cloudBitrateMode,
                fallback: StreamingBitrateMode.auto.rawValue,
                parse: StreamingBitrateMode.init(rawValue:)
            ),
            cloudBitrateMbps: integerValue(
                key: AppSettingsStore.cloudBitrateMbpsKey,
                cached: cached?.cloudBitrateMbps,
                fallback: 20,
                range: 1 ... 200
            ),
            audioBitrateMode: enumRawValue(
                key: AppSettingsStore.audioBitrateModeKey,
                cached: cached?.audioBitrateMode,
                fallback: StreamingBitrateMode.auto.rawValue,
                parse: StreamingBitrateMode.init(rawValue:)
            ),
            audioBitrateKbps: integerValue(
                key: AppSettingsStore.audioBitrateKbpsKey,
                cached: cached?.audioBitrateKbps,
                fallback: 20,
                range: 1 ... 512
            ),
            homeTurnFallbackEnabled: boolValue(
                key: AppSettingsStore.homeTurnFallbackKey,
                cached: cached?.homeTurnFallbackEnabled,
                fallback: true
            )
        )
    }

    func save(_ snapshot: AppSettingsSnapshotV1) {
        if let data = try? JSONEncoder().encode(snapshot) {
            defaults.set(data, forKey: Self.snapshotKey)
        }
        defaults.set(snapshot.appearanceMode, forKey: AppSettingsStore.appearanceModeKey)
        defaults.set(snapshot.appIconPreset, forKey: AppSettingsStore.appIconPresetKey)
        defaults.set(snapshot.cloudRegionPreset, forKey: AppSettingsStore.cloudRegionKey)
        defaults.set(snapshot.usesEphemeralLoginSession, forKey: AppSettingsStore.ephemeralLoginKey)
        defaults.set(snapshot.preferredGameLocale, forKey: AppSettingsStore.preferredGameLocaleKey)
        defaults.set(snapshot.cloudResolution, forKey: AppSettingsStore.cloudResolutionKey)
        defaults.set(snapshot.homeResolution, forKey: AppSettingsStore.homeResolutionKey)
        defaults.set(snapshot.preferIPv6, forKey: AppSettingsStore.preferIPv6Key)
        defaults.set(snapshot.codecPreference, forKey: AppSettingsStore.codecPreferenceKey)
        defaults.set(snapshot.homeBitrateMode, forKey: AppSettingsStore.homeBitrateModeKey)
        defaults.set(snapshot.homeBitrateMbps, forKey: AppSettingsStore.homeBitrateMbpsKey)
        defaults.set(snapshot.cloudBitrateMode, forKey: AppSettingsStore.cloudBitrateModeKey)
        defaults.set(snapshot.cloudBitrateMbps, forKey: AppSettingsStore.cloudBitrateMbpsKey)
        defaults.set(snapshot.audioBitrateMode, forKey: AppSettingsStore.audioBitrateModeKey)
        defaults.set(snapshot.audioBitrateKbps, forKey: AppSettingsStore.audioBitrateKbpsKey)
        defaults.set(snapshot.homeTurnFallbackEnabled, forKey: AppSettingsStore.homeTurnFallbackKey)
    }

    private func stringValue(key: String, cached: String?, fallback: String) -> String {
        defaults.object(forKey: key) == nil
            ? cached ?? fallback
            : defaults.string(forKey: key) ?? fallback
    }

    private func enumRawValue<T>(
        key: String,
        cached: String?,
        fallback: String,
        parse: (String) -> T?
    ) -> String {
        let candidate = stringValue(key: key, cached: cached, fallback: fallback)
        return parse(candidate) == nil ? fallback : candidate
    }

    private func boolValue(key: String, cached: Bool?, fallback: Bool) -> Bool {
        guard let object = defaults.object(forKey: key) else { return cached ?? fallback }
        guard let number = object as? NSNumber,
              String(cString: number.objCType) == "c"
        else { return fallback }
        return number.boolValue
    }

    private func integerValue(
        key: String,
        cached: Int?,
        fallback: Int,
        range: ClosedRange<Int>
    ) -> Int {
        let candidate: Int
        if let object = defaults.object(forKey: key) {
            guard let number = object as? NSNumber,
                  String(cString: number.objCType) != "c",
                  number.doubleValue.rounded(.towardZero) == number.doubleValue
            else { return fallback }
            candidate = number.intValue
        } else {
            candidate = cached ?? fallback
        }
        return min(range.upperBound, max(range.lowerBound, candidate))
    }

    private func resolutionValue(
        key: String,
        cached: Int?,
        fallback: Int,
        allowed: Set<Int>
    ) -> Int {
        let candidate: Int
        if let object = defaults.object(forKey: key) {
            guard let number = object as? NSNumber,
                  String(cString: number.objCType) != "c",
                  number.doubleValue.rounded(.towardZero) == number.doubleValue
            else { return fallback }
            candidate = number.intValue
        } else {
            candidate = cached ?? fallback
        }
        return allowed.contains(candidate) ? candidate : fallback
    }
}
