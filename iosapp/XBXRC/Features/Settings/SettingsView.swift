import SwiftUI
import UIKit

struct SettingsView: View {
    @EnvironmentObject private var settingsStore: AppSettingsStore
    @EnvironmentObject private var authStore: AuthStore
    @EnvironmentObject private var cloudStore: CloudLibraryStore

    @State private var selectedRegion: CloudRegionPreset = .default
    @State private var traceProfile = IOSRuntimeTrace.currentProfile
    @State private var traceExportItem: TraceExportItem?
    @State private var traceActionError: String?
    @State private var regionActionError: String?
    @State private var isPreparingTraceExport = false
    @State private var isApplyingRegion = false
    @State private var showsTraceClearConfirmation = false

    var body: some View {
        NavigationStack {
            Form {
                accountSection
                cloudSection
                diagnosticsSection
                aboutSection
            }
            .scrollContentBackground(.hidden)
            .appThemeCanvas()
            .navigationTitle("设置")
            .navigationBarTitleDisplayMode(.large)
            .onAppear {
                selectedRegion = settingsStore.cloudRegionPreset
            }
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
                    get: { traceActionError != nil || regionActionError != nil },
                    set: { presented in
                        if !presented {
                            traceActionError = nil
                            regionActionError = nil
                        }
                    }
                )
            ) {
                Button("知道了", role: .cancel) {}
            } message: {
                Text(regionActionError ?? traceActionError ?? "设置暂时不可用")
            }
        }
    }

    private var accountSection: some View {
        Section {
            Toggle(
                "使用无 Cookie 临时会话",
                isOn: $settingsStore.usesEphemeralLoginSession
            )
        } header: {
            Text("登录")
        } footer: {
            Text("开启后，下一次登录不会复用 Microsoft 登录 Cookie 和上次账号信息。现有 Xbox 会话保持有效，退出后重新登录生效。")
        }
    }

    private var cloudSection: some View {
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
            .disabled(isApplyingRegion || selectedRegion == settingsStore.cloudRegionPreset)
        } header: {
            Text("云游戏")
        } footer: {
            Text("地区路由会影响 Xbox streaming token 与云游戏区域选择。应用后会刷新当前会话和游戏库。")
        }
    }

    private var diagnosticsSection: some View {
        Section("诊断") {
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

    private var aboutSection: some View {
        Section("关于") {
            LabeledContent("应用", value: "XBXRC")
            LabeledContent("版本", value: versionText)
        }
    }

    private var cloudAccessStatus: String {
        guard let session = authStore.session else { return "未登录" }
        switch session.appLevel {
        case 2...: return "可用"
        case 1: return "地区受限"
        default: return "等待刷新"
        }
    }

    private var versionText: String {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString")
            as? String ?? "unknown"
        let build = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion")
            as? String ?? "unknown"
        return "\(version) (\(build))"
    }

    private func applyRegion() {
        guard !isApplyingRegion else { return }
        isApplyingRegion = true
        regionActionError = nil
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
                } else {
                    regionActionError = authStore.errorMessage ?? "当前地区无法建立云游戏访问"
                }
            }
            isApplyingRegion = false
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
                traceActionError = CloudLibraryDiagnostics.safeError(error)
            }
            isPreparingTraceExport = false
        }
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
