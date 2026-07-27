import SwiftUI
import UIKit

struct MySettingsPresentation: Equatable {
    let cloudAccessStatus: String
    let cloudGamingSummary: String
    let loginMode: String
    let traceSummary: String
    let version: String

    init(
        appLevel: Int?,
        cloudRegionTitle: String,
        usesEphemeralLoginSession: Bool,
        traceProfileTitle: String,
        version: String
    ) {
        let cloudAccessStatus = Self.cloudAccessStatus(for: appLevel)
        self.cloudAccessStatus = cloudAccessStatus
        cloudGamingSummary = "\(cloudRegionTitle) · \(cloudAccessStatus)"
        loginMode = usesEphemeralLoginSession ? "无 Cookie 临时会话" : "标准会话"
        traceSummary = traceProfileTitle
        self.version = "XBXRC \(version)"
    }

    static func cloudAccessStatus(for appLevel: Int?) -> String {
        guard let appLevel else { return "未登录" }
        switch appLevel {
        case 2...: return "可用"
        case 1: return "地区受限"
        default: return "等待刷新"
        }
    }
}

struct AppearanceSettingsView: View {
    @EnvironmentObject private var settingsStore: AppSettingsStore

    @State private var isApplyingIcon = false
    @State private var applyingIcon: AppIconPreset?
    @State private var actionError: String?

    var body: some View {
        Form {
            Section {
                Picker("应用外观", selection: $settingsStore.appearanceMode) {
                    ForEach(AppAppearanceMode.allCases) { mode in
                        Text(mode.title).tag(mode)
                    }
                }
                .pickerStyle(.segmented)
            } header: {
                Text("外观")
            } footer: {
                Text("跟随系统会根据设备的外观自动切换。选择会立即生效并在下次启动恢复。")
            }

            Section {
                ForEach(AppIconPreset.allCases) { preset in
                    Button {
                        applyIcon(preset)
                    } label: {
                        HStack(spacing: 12) {
                            Image(systemName: preset.systemImage)
                                .font(.title3.weight(.semibold))
                                .foregroundStyle(AppThemePalette.brand)
                                .frame(width: 28)

                            VStack(alignment: .leading, spacing: 2) {
                                Text(preset.title)
                                    .foregroundStyle(.primary)
                                Text(preset.description)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }

                            Spacer(minLength: 8)
                            if applyingIcon == preset {
                                ProgressView()
                                    .controlSize(.small)
                            } else if settingsStore.appIconPreset == preset {
                                Image(systemName: "checkmark.circle.fill")
                                    .foregroundStyle(AppThemePalette.brand)
                            }
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .disabled(isApplyingIcon || settingsStore.appIconPreset == preset)
                }
            } header: {
                Text("应用图标")
            } footer: {
                Text("图标切换由系统完成。设备或构建环境不支持时，当前图标和设置会保持不变。")
            }
        }
        .scrollContentBackground(.hidden)
        .appThemeCanvas()
        .navigationTitle("外观与图标")
        .navigationBarTitleDisplayMode(.inline)
        .alert(
            "设置操作失败",
            isPresented: Binding(
                get: { actionError != nil },
                set: { presented in
                    if !presented {
                        actionError = nil
                    }
                }
            )
        ) {
            Button("知道了", role: .cancel) {}
        } message: {
            Text(actionError ?? "设置暂时不可用")
        }
    }

    private func applyIcon(_ preset: AppIconPreset) {
        guard !isApplyingIcon else { return }
        isApplyingIcon = true
        applyingIcon = preset
        actionError = nil
        Task {
            actionError = await settingsStore.setAppIconPreset(preset)
            applyingIcon = nil
            isApplyingIcon = false
        }
    }
}

struct CloudGamingSettingsView: View {
    @EnvironmentObject private var settingsStore: AppSettingsStore
    @EnvironmentObject private var authStore: AuthStore
    @EnvironmentObject private var cloudStore: CloudLibraryStore

    @Binding var isApplyingRegion: Bool
    @State private var selectedRegion: CloudRegionPreset = .default
    @State private var actionError: String?

    var body: some View {
        Form {
            Section {
                Picker("地区路由", selection: $selectedRegion) {
                    ForEach(CloudRegionPreset.allCases) { preset in
                        Text(preset.title).tag(preset)
                    }
                }

                LabeledContent("云访问状态", value: cloudAccessStatus)

                Button {
                    applyRegion()
                } label: {
                    HStack {
                        Label("应用地区设置", systemImage: "network")
                        Spacer()
                        if isApplyingRegion {
                            ProgressView().controlSize(.small)
                        }
                    }
                }
                .disabled(isApplyingRegion)
            } header: {
                Text("地区路由")
            } footer: {
                Text("地区路由会影响 Xbox streaming token 与云游戏区域选择。应用后会刷新当前会话和游戏库。")
            }

            Section {
                Picker("游戏语言", selection: $settingsStore.preferredGameLocale) {
                    ForEach(PreferredGameLanguageCatalog.all) { option in
                        Text(option.title).tag(option.code)
                    }
                }

                Picker("云游戏分辨率", selection: $settingsStore.cloudResolution) {
                    ForEach(StreamingResolutionOption.cloudChoices) { option in
                        Text(option.title).tag(option)
                    }
                }

                Picker("主机串流分辨率", selection: $settingsStore.homeResolution) {
                    ForEach(StreamingResolutionOption.homeChoices) { option in
                        Text(option.title).tag(option)
                    }
                }
            } header: {
                Text("会话偏好")
            } footer: {
                Text("这些设置会在下一次启动云游戏或主机串流时生效。")
            }

            Section {
                Toggle("优先 IPv6", isOn: $settingsStore.preferIPv6)

                Picker("视频 Codec 档位", selection: $settingsStore.codecPreference) {
                    ForEach(StreamingCodecOption.allCases) { option in
                        Text("\(option.title) · \(option.summary)").tag(option)
                    }
                }

                Toggle("允许 xHome 使用 TURN 中继", isOn: $settingsStore.homeTurnFallbackEnabled)
            } header: {
                Text("网络与协商")
            } footer: {
                Text("这组设置会直接进入共享 Rust plan，影响候选排序、视频编码档位和 xHome 的中继兜底。")
            }

            Section {
                Picker("主机视频码率模式", selection: $settingsStore.homeBitrateMode) {
                    ForEach(StreamingBitrateMode.allCases) { mode in
                        Text(mode.title).tag(mode)
                    }
                }
                if settingsStore.homeBitrateMode == .custom {
                    Stepper(value: $settingsStore.homeBitrateMbps, in: 1...200) {
                        LabeledContent("主机视频上限", value: "\(settingsStore.homeBitrateMbps) Mb/s")
                    }
                }

                Picker("云游戏视频码率模式", selection: $settingsStore.cloudBitrateMode) {
                    ForEach(StreamingBitrateMode.allCases) { mode in
                        Text(mode.title).tag(mode)
                    }
                }
                if settingsStore.cloudBitrateMode == .custom {
                    Stepper(value: $settingsStore.cloudBitrateMbps, in: 1...200) {
                        LabeledContent("云游戏视频上限", value: "\(settingsStore.cloudBitrateMbps) Mb/s")
                    }
                }

                Picker("音频码率模式", selection: $settingsStore.audioBitrateMode) {
                    ForEach(StreamingBitrateMode.allCases) { mode in
                        Text(mode.title).tag(mode)
                    }
                }
                if settingsStore.audioBitrateMode == .custom {
                    Stepper(value: $settingsStore.audioBitrateKbps, in: 1...512) {
                        LabeledContent("音频上限", value: "\(settingsStore.audioBitrateKbps) kb/s")
                    }
                }
            } header: {
                Text("码率上限")
            } footer: {
                Text("自动模式沿共享策略默认值运行；手动模式会把上限带入云游戏、主机串流和 SDP 音频码率。")
            }
        }
        .scrollContentBackground(.hidden)
        .appThemeCanvas()
        .navigationTitle("云游戏与串流")
        .navigationBarTitleDisplayMode(.inline)
        .navigationBarBackButtonHidden(isApplyingRegion)
        .onAppear {
            selectedRegion = settingsStore.cloudRegionPreset
        }
        .alert(
            "设置操作失败",
            isPresented: Binding(
                get: { actionError != nil },
                set: { presented in
                    if !presented {
                        actionError = nil
                    }
                }
            )
        ) {
            Button("知道了", role: .cancel) {}
        } message: {
            Text(actionError ?? "设置暂时不可用")
        }
    }

    private var cloudAccessStatus: String {
        MySettingsPresentation.cloudAccessStatus(
            for: authStore.session.map { Int($0.appLevel) }
        )
    }

    private func applyRegion() {
        guard !isApplyingRegion else { return }
        isApplyingRegion = true
        actionError = nil
        let preset = selectedRegion
        settingsStore.setCloudRegionPreset(preset)
        Task {
            await cloudStore.clear()
            if authStore.isSignedIn {
                let renewed = await authStore.refreshForRegionChange()
                if renewed {
                    await cloudStore.activate(session: authStore.session) {
                        try await authStore.prepareCloudAccess()
                    }
                    actionError = cloudStore.errorMessage
                } else {
                    actionError = authStore.errorMessage ?? "当前地区无法建立云游戏访问"
                }
            }
            isApplyingRegion = false
        }
    }
}

struct LoginPreferencesView: View {
    @EnvironmentObject private var settingsStore: AppSettingsStore

    var body: some View {
        Form {
            Section {
                Toggle(
                    "使用无 Cookie 临时会话",
                    isOn: $settingsStore.usesEphemeralLoginSession
                )
            } footer: {
                Text("开启后，下一次登录不会复用 Microsoft 登录 Cookie 和上次账号信息。现有 Xbox 会话保持有效，退出后重新登录生效。")
            }
        }
        .scrollContentBackground(.hidden)
        .appThemeCanvas()
        .navigationTitle("登录偏好")
        .navigationBarTitleDisplayMode(.inline)
    }
}

struct DiagnosticsSettingsView: View {
    @Binding var traceProfile: IOSRuntimeTraceProfile

    @State private var traceExportItem: TraceExportItem?
    @State private var actionError: String?
    @State private var isPreparingTraceExport = false
    @State private var showsTraceClearConfirmation = false

    var body: some View {
        Form {
            Section {
                Picker("Trace 记录级别", selection: $traceProfile) {
                    ForEach(IOSRuntimeTraceProfile.allCases) { profile in
                        Text(profile.title).tag(profile)
                    }
                }

                Button("导出当前 Trace", systemImage: "square.and.arrow.up") {
                    prepareTraceExport(allFiles: false)
                }
                .disabled(isPreparingTraceExport)

                Button("导出全部 Trace", systemImage: "archivebox") {
                    prepareTraceExport(allFiles: true)
                }
                .disabled(isPreparingTraceExport)

                Button("清理 Trace", systemImage: "trash", role: .destructive) {
                    showsTraceClearConfirmation = true
                }
            }
        }
        .scrollContentBackground(.hidden)
        .appThemeCanvas()
        .navigationTitle("诊断")
        .navigationBarTitleDisplayMode(.inline)
        .onChange(of: traceProfile) { _, profile in
            IOSRuntimeTrace.setProfile(profile)
        }
        .confirmationDialog(
            "清理全部 iOS Trace？",
            isPresented: $showsTraceClearConfirmation,
            titleVisibility: .visible
        ) {
            Button("清理 Trace", role: .destructive) {
                Task { await IOSRuntimeTrace.clearFiles() }
            }
        } message: {
            Text("历史诊断文件会被删除，当前 profile 会继续生效。")
        }
        .sheet(item: $traceExportItem) { item in
            TraceShareSheet(url: item.url)
        }
        .alert(
            "设置操作失败",
            isPresented: Binding(
                get: { actionError != nil },
                set: { presented in
                    if !presented {
                        actionError = nil
                    }
                }
            )
        ) {
            Button("知道了", role: .cancel) {}
        } message: {
            Text(actionError ?? "设置暂时不可用")
        }
    }

    private func prepareTraceExport(allFiles: Bool) {
        guard !isPreparingTraceExport else { return }
        isPreparingTraceExport = true
        Task {
            do {
                IOSRuntimeTrace.event(
                    domain: "trace",
                    event: "exportRequested",
                    payload: ["allFiles": .bool(allFiles)],
                    dimension: .core,
                    importance: .essential
                )
                let url = try await IOSRuntimeTrace.prepareExport(allFiles: allFiles)
                traceExportItem = TraceExportItem(url: url)
            } catch {
                actionError = CloudLibraryDiagnostics.safeError(error)
            }
            isPreparingTraceExport = false
        }
    }
}

struct AboutSettingsView: View {
    var body: some View {
        Form {
            Section {
                LabeledContent("应用", value: "XBXRC")
                LabeledContent("版本", value: versionText)
            }
        }
        .scrollContentBackground(.hidden)
        .appThemeCanvas()
        .navigationTitle("关于")
        .navigationBarTitleDisplayMode(.inline)
    }

    private var versionText: String {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString")
            as? String ?? "unknown"
        let build = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion")
            as? String ?? "unknown"
        return "\(version) (\(build))"
    }
}

private struct TraceExportItem: Identifiable {
    let id = UUID()
    let url: URL
}

private struct TraceShareSheet: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context _: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: [url], applicationActivities: nil)
    }

    func updateUIViewController(
        _: UIActivityViewController,
        context _: Context
    ) {}
}
