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
        AppSettingsForm {
            Section {
                AppSettingsPicker(
                    "应用外观", selection: $settingsStore.appearanceMode,
                    selectedTitle: settingsStore.appearanceMode.title
                ) {
                    ForEach(AppAppearanceMode.allCases) { mode in
                        Text(LocalizedStringKey(mode.title)).tag(mode)
                    }
                }
                .appSettingsHint("跟随系统外观，立即生效并保存", identifier: "appearance-theme")
            } header: {
                Text("外观")
            }

            Section {
                ForEach(AppIconPreset.allCases) { preset in
                    Button {
                        applyIcon(preset)
                    } label: {
                        HStack(spacing: 12) {
                            Image(systemName: preset.systemImage)
                                .appFont(.title, weight: .semibold)
                                .foregroundStyle(AppThemePalette.brand)
                                .frame(width: 28)

                            VStack(alignment: .leading, spacing: 2) {
                                Text(LocalizedStringKey(preset.title))
                                    .foregroundStyle(.primary)
                                Text(LocalizedStringKey(preset.description))
                                    .appFont(.caption, weight: .regular)
                                    .foregroundStyle(AppThemePalette.textSecondary)
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
                    .buttonStyle(AppSettingsRowButtonStyle(inset: 16))
                    .containerValue(\.appSettingsRowOwnsPadding, true)
                    .disabled(isApplyingIcon || settingsStore.appIconPreset == preset)
                }
            } header: {
                Text("应用图标")
            }
        }
        .scrollContentBackground(.hidden)
        .appThemeCanvas()
        .appPageToolbar("外观与图标")
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
            Text(LocalizedStringKey(actionError ?? "设置暂时不可用"))
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

struct DiagnosticsSettingsView: View {
    @Environment(\.locale) private var locale
    @Binding var traceProfile: IOSRuntimeTraceProfile

    @State private var traceExportItem: TraceExportItem?
    @State private var actionError: String?
    @State private var isPreparingTraceExport = false
    @State private var showsTraceClearConfirmation = false
    @AppStorage("ios.diagnostics.lightweight") private var lightweight = false
    @State private var records: [IOSDiagnosticRecord] = []
    @State private var now = Date()
    @State private var capturing = false
    private var activeRecord: IOSDiagnosticRecord? { records.first { $0.isActive } }

    var body: some View {
        AppSettingsForm {
            Section {
                AppSettingsToggle("轻量后台记录", isOn: $lightweight)
                LabeledContent {
                    if activeRecord == nil { Text("就绪") } else { Text("正在录制 \(remainingTime)") }
                } label: {
                    Text("记录状态")
                }
                .accessibilityIdentifier("diagnostic-record-status").appSettingsInformation()
                AppSettingsActionButton(activeRecord == nil ? "开始诊断录制" : "停止并导出") {
                    toggleCapture()
                }
                .disabled(capturing || isPreparingTraceExport)
                .appSettingsHint("离开页面后继续录制", identifier: "diagnostics-recording")
                AppFactGroup(facts: [("单次上限", "5 分钟"), ("保留记录", "3 份"), ("自动清理", "7 天")])
                    .appSettingsInformation()
            } header: {
                Text("诊断录制")
            }
            Section {
                if records.isEmpty {
                    Text("完成录制后，记录会显示在这里。").appFont(.caption).appSettingsInformation()
                }
                ForEach(records) { record in
                    Button {
                        exportRecord(record)
                    } label: {
                        HStack(spacing: 12) {
                            AppIconView(.pulse, size: 22)
                            VStack(alignment: .leading, spacing: 4) {
                                HStack(spacing: 8) {
                                    Text(
                                        LocalizedStringKey(
                                            record.operation == nil ? "手动录制" : "问题诊断"))
                                    Text(
                                        LocalizedStringKey(
                                            record.isActive
                                                ? "录制中" : record.endedAt == nil ? "已中断" : "已完成")
                                    )
                                    .appFont(.caption).foregroundStyle(
                                        AppThemePalette.textSecondary)
                                }
                                Text(
                                    record.startedAt.formatted(
                                        Date.FormatStyle(date: .abbreviated, time: .shortened)
                                            .locale(locale))
                                )
                                .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
                            }
                            Spacer(minLength: 8)
                            AppIconView(.share, size: 22)
                        }
                        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                    }
                    .accessibilityHint("导出诊断记录")
                    .buttonStyle(AppSettingsRowButtonStyle(inset: 16))
                    .containerValue(\.appSettingsRowOwnsPadding, true)
                    .disabled(capturing || isPreparingTraceExport)
                }
            } header: {
                Text("最近记录")
            }
            Section {
                AppSettingsPicker(
                    "Trace 记录级别", selection: $traceProfile, selectedTitle: traceProfile.title
                ) {
                    ForEach(IOSRuntimeTraceProfile.allCases) { profile in
                        Text(LocalizedStringKey(profile.title)).tag(profile)
                    }
                }

                AppSettingsActionButton("导出当前 Trace", systemImage: "square.and.arrow.up") {
                    prepareTraceExport(allFiles: false)
                }
                .disabled(isPreparingTraceExport)

                AppSettingsActionButton("导出全部 Trace", systemImage: "archivebox") {
                    prepareTraceExport(allFiles: true)
                }
                .disabled(isPreparingTraceExport)

                AppSettingsActionButton("清理 Trace", systemImage: "trash", role: .destructive) {
                    showsTraceClearConfirmation = true
                }
            }
        }
        .scrollContentBackground(.hidden)
        .appThemeCanvas()
        .onChange(of: traceProfile) { _, profile in
            IOSRuntimeTrace.setProfile(profile)
        }
        .onChange(of: lightweight) { _, value in IOSRuntimeTrace.setLightweight(value) }
        .task {
            while !Task.isCancelled {
                records = await IOSRuntimeTrace.diagnosticRecords()
                now = Date()
                try? await Task.sleep(for: .seconds(1))
            }
        }
        .confirmationDialog(
            "清理全部 iOS Trace？",
            isPresented: $showsTraceClearConfirmation,
            titleVisibility: .visible
        ) {
            Button("清理 Trace", role: .destructive) {
                Task {
                    await IOSRuntimeTrace.clearFiles()
                    records = await IOSRuntimeTrace.diagnosticRecords()
                }
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
            Text(LocalizedStringKey(actionError ?? "设置暂时不可用"))
        }
    }

    private var remainingTime: String {
        let seconds = max(0, 300 - Int(now.timeIntervalSince(activeRecord?.startedAt ?? now)))
        return "\(seconds / 60):\(String(format: "%02d", seconds % 60))"
    }

    private func toggleCapture() {
        guard !capturing else { return }
        capturing = true
        Task {
            do {
                if let record = activeRecord {
                    _ = await IOSRuntimeTrace.stopCapture()
                    traceExportItem = TraceExportItem(
                        url: try await IOSRuntimeTrace.diagnosticExport(record))
                } else {
                    try await IOSRuntimeTrace.startCapture()
                }
                actionError = nil
            } catch { actionError = "诊断操作失败，请重试。" }
            records = await IOSRuntimeTrace.diagnosticRecords()
            capturing = false
        }
    }

    private func exportRecord(_ record: IOSDiagnosticRecord) {
        guard !isPreparingTraceExport else { return }
        isPreparingTraceExport = true
        Task {
            do {
                traceExportItem = TraceExportItem(
                    url: try await IOSRuntimeTrace.diagnosticExport(record))
            } catch { actionError = "诊断导出失败，请重试。" }
            isPreparingTraceExport = false
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
    @Environment(\.openURL) private var openURL
    var body: some View {
        AppSettingsForm {
            Section {
                AppFactGroup(facts: [("应用", "XBXRC"), ("版本", versionText)])
                    .appSettingsInformation()
            }
            Section("应用更新") {
                AppSettingsExternalLink(
                    "查看版本与更新说明",
                    destination: URL(string: "https://github.com/xuasir/xbxrc/releases")!)
                Text("通过安装应用时的分发渠道更新。TestFlight 版本可在 TestFlight 中查看新构建。")
                    .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
                    .appSettingsInformation()
            }
            Section("支持") {
                AppSettingsExternalLink(
                    "项目主页", destination: URL(string: "https://github.com/xuasir/xbxrc")!)
            }
        }
        .scrollContentBackground(.hidden)
        .appThemeCanvas()
    }

    private var versionText: String {
        let version =
            Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString")
            as? String ?? "unknown"
        let build =
            Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion")
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
