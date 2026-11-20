import AudioToolbox
import Metal
import SwiftUI
import UIKit

#if !targetEnvironment(simulator)
    import MetalFX
#endif

/// 数值通过拖动、预设与步进调整，完整操作区适配触屏与大字号。
struct MobileIntegerSetting: View {
    let title: String
    @Binding var value: Int
    let range: ClosedRange<Int>
    let unit: String
    var presets: [Int] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            LabeledContent {
                Text("\(value) \(unit)")
            } label: {
                Text(LocalizedStringKey(title))
            }
            Slider(
                value: Binding(get: { Double(value) }, set: { value = Int($0.rounded()) }),
                in: Double(range.lowerBound)...Double(range.upperBound), step: 1
            )
            .frame(minHeight: 44)
            .accessibilityLabel(Text(LocalizedStringKey(title)))
            .accessibilityValue("\(value) \(unit)")
            if !presets.isEmpty {
                ViewThatFits(in: .horizontal) {
                    HStack { presetButtons }
                    ScrollView(.horizontal) { HStack { presetButtons } }
                }
            }
            Stepper("精细调整", value: $value, in: range)
                .frame(minHeight: 44)
                .accessibilityLabel(Text("\(Text(LocalizedStringKey(title)))，精细调整"))
        }
    }

    private var presetButtons: some View {
        ForEach(presets, id: \.self) { preset in
            Button {
                value = preset
            } label: {
                Text("\(preset)").appFont(.caption)
                    .frame(minWidth: 44, minHeight: 44)
                    .foregroundStyle(
                        value == preset ? AppThemePalette.brand : AppThemePalette.textPrimary
                    )
                    .appSurface(radius: 6)
            }
            .accessibilityLabel(Text("\(Text(LocalizedStringKey(title))) \(preset) \(unit)"))
            .accessibilityAddTraits(value == preset ? .isSelected : [])
        }
    }
}

struct MobilePictureSettingsView: View {
    @Binding var picture: MobilePicturePreferences
    var videoContentMode: Binding<StreamingVideoContentMode>? = nil
    var onSaveDefault: (() -> Void)? = nil
    var onRestoreDefault: (() -> Void)? = nil
    var body: some View {
        AppSettingsForm {
            Section {
                if let videoContentMode {
                    Picker("画面比例", selection: videoContentMode) {
                        ForEach(StreamingVideoContentMode.allCases, id: \.self) { mode in
                            Text(mode.title).tag(mode)
                        }
                    }.pickerStyle(.menu)
                }
                AppSettingsToggle("兼容显示", isOn: $picture.compatibilityMode)
                    .appSettingsHint("使用原始画面，保留增强偏好", identifier: "picture-compatibility")
            } header: {
                Text("显示方式")
            }
            Section {
                AppSettingsToggle("超分辨率", isOn: $picture.superResolution)
                    .disabled(!supportsSuperResolution)
                    .appSettingsHint(
                        supportsSuperResolution ? "放大时增强清晰度" : "当前设备使用标准画面缩放。",
                        identifier: "picture-enhancement")
                AppSettingsPicker(
                    "锐化", selection: $picture.sharpness,
                    selectedTitle: picture.sharpness == 0
                        ? "关闭" : picture.sharpness == 1 ? "柔和" : "清晰"
                ) {
                    Text("关闭").tag(0)
                    Text("柔和").tag(1)
                    Text("清晰").tag(3)
                }
            } header: {
                Text("画质增强")
            }
            .disabled(picture.compatibilityMode)
            Section {
                AppSettingsPicker(
                    "色彩风格",
                    selection: Binding(
                        get: { picture.colorPreset?.rawValue ?? "custom" },
                        set: {
                            if let preset = MobileColorPreset(rawValue: $0) {
                                picture.apply(preset)
                            }
                        }
                    ), selectedTitle: picture.colorPreset?.title ?? "自定义"
                ) {
                    ForEach(MobileColorPreset.allCases) {
                        Text(LocalizedStringKey($0.title)).tag($0.rawValue)
                    }
                    if picture.colorPreset == nil { Text("自定义").tag("custom") }
                }
                colorSlider("饱和度", value: $picture.saturation)
                colorSlider("对比度", value: $picture.contrast)
                colorSlider("亮度", value: $picture.brightness)
            } header: {
                Text("色彩")
            }
            .disabled(picture.compatibilityMode)
            Section {
                AppSettingsActionButton("恢复标准画面") { picture = MobilePicturePreferences() }
                if let onRestoreDefault {
                    AppSettingsActionButton("恢复应用默认画面", action: onRestoreDefault)
                }
                if let onSaveDefault {
                    AppSettingsActionButton("保存为下次串流默认画面", action: onSaveDefault)
                }
            }
        }
        .appPageToolbar("画面调节")
    }

    private var supportsSuperResolution: Bool {
        #if targetEnvironment(simulator)
            return false
        #else
            guard let device = MTLCreateSystemDefaultDevice() else { return false }
            return MTLFXSpatialScalerDescriptor.supportsDevice(device)
        #endif
    }

    private func colorSlider(_ title: String, value: Binding<Double>) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            LabeledContent {
                Text("\(Int(value.wrappedValue))%")
            } label: {
                Text(LocalizedStringKey(title))
            }
            Slider(value: value, in: 0...200, step: 1)
                .frame(minHeight: 44).accessibilityLabel(Text(LocalizedStringKey(title)))
        }
    }
}

@MainActor
enum MobileInteractionFeedback {
    private static var lastFeedback = Date.distantPast
    static func play(_ preferences: MobilePreferences) {
        guard Date().timeIntervalSince(lastFeedback) >= 0.15 else { return }
        lastFeedback = Date()
        if preferences.uiHaptics { UISelectionFeedbackGenerator().selectionChanged() }
        if preferences.uiAudio { AudioServicesPlaySystemSound(1104) }
    }
}
