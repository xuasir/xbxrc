import SwiftUI

struct OfficialTouchLayoutPreviewView: View {
    let bundle: any OfficialTouchBundleProviding
    let choice: XboxTouchLayoutChoice
    let gameName: String
    let ownerGeneration: UInt64
    @Environment(\.dismiss) private var dismiss
    @Environment(\.locale) private var locale
    @EnvironmentObject private var auth: AuthStore
    @EnvironmentObject private var library: SavedTouchLayoutsStore
    @EnvironmentObject private var settings: AppSettingsStore
    @State private var prepared: PreparedTouchLayout?
    @State private var orientation: TouchLayoutOrientation = .portrait
    @State private var previewError: SectionRequestFailure?
    @State private var saveError: String?
    @State private var saving = false
    @State private var saveTask: Task<Void, Never>?
    @State private var previewRevision = 0
    private var canSave: Bool { prepared?.layout.report.isSupported == true }

    var body: some View {
        Group {
            if let prepared {
                TouchLayoutWorkspace(
                    title: "布局预览", saveIdentifier: "official-touch-preview-save",
                    onSave: saveAndClose, orientation: $orientation,
                    appearance: .init(
                        opacity: settings.mobile.touchOpacity, scale: settings.mobile.touchScale),
                    configuration: .init(identity: prepared.id, scale: settings.mobile.touchScale),
                    preparedLayout: prepared, onBack: saveAndClose,
                    backIdentifier: "official-touch-preview-close",
                    backLabel: canSave ? "保存并退出" : "退出预览", saving: saving,
                    saveDisabled: !canSave || library.isSaving,
                    backDisabled: canSave && library.isSaving
                ) {
                    LabeledContent("游戏") { Text(verbatim: gameName) }
                    LabeledContent("布局") { Text(verbatim: choice.name) }
                    LabeledContent("版本") { Text(verbatim: bundle.version) }
                    if let author = prepared.layout.author, !author.isEmpty {
                        Text(verbatim: appLocalizedFormat("作者 %@", locale: locale, author))
                            .appFont(.caption)
                    }
                    Text("触摸按键体验布局").appFont(.caption)
                    if !canSave {
                        Text("布局等待适配").appFont(.bodySmall)
                        ForEach(
                            prepared.layout.report.issues.filter { $0.severity == .unsupported }
                        ) {
                            Text(verbatim: $0.message).appFont(.caption)
                        }
                    }
                }
            } else {
                loadingPreview
            }
        }
        .environment(\.colorScheme, .dark).preferredColorScheme(.dark)
        .interactiveDismissDisabled()
        .task(id: previewRevision) { await preparePreview() }
        .onChange(of: auth.ownerGeneration) { _, _ in
            saveTask?.cancel()
            dismiss()
        }
        .onDisappear { saveTask?.cancel() }
        .alert(
            "保存布局失败",
            isPresented: Binding(
                get: { saveError != nil }, set: { if !$0 { saveError = nil } }
            )
        ) {
            Button("重试", action: saveAndClose)
            Button("完成", role: .cancel) { saveError = nil }
        } message: {
            Text(verbatim: saveError ?? "")
        }
    }

    private var loadingPreview: some View {
        ZStack(alignment: .topLeading) {
            Color(white: 0.06).ignoresSafeArea()
            Group {
                if let previewError {
                    AppSectionRequestState(
                        title: "布局预览",
                        status: SectionRequestStatus(phase: .failed, failure: previewError),
                        identifier: "official-touch-preview"
                    ) { previewRevision += 1 }
                } else {
                    ProgressView("正在准备布局预览")
                }
            }.padding(24).frame(maxWidth: .infinity, maxHeight: .infinity)
            Button {
                dismiss()
            } label: {
                AppIconView(.chevron, size: 18).rotationEffect(.degrees(180))
                    .frame(width: 44, height: 44)
            }.buttonStyle(.plain).padding(16).accessibilityLabel("退出预览")
                .accessibilityIdentifier("official-touch-preview-close")
        }
        .appResponsiveTypography().foregroundStyle(AppThemePalette.textPrimary)
        .tint(AppThemePalette.brand)
    }

    private func preparePreview() async {
        previewError = nil
        guard ownerGeneration == auth.ownerGeneration else {
            dismiss()
            return
        }
        do {
            let snapshot = try await bundle.preview(layoutID: choice.id)
            let result = try await PreparedTouchLayout.prepare(snapshot)
            try Task.checkCancellation()
            guard ownerGeneration == auth.ownerGeneration else { return }
            prepared = result
        } catch {
            guard !Task.isCancelled, !SectionRequestFailure.isCancellation(error) else { return }
            previewError = SectionRequestFailure(error: error)
        }
    }

    private func saveAndClose() {
        guard !saving, ownerGeneration == auth.ownerGeneration else { return }
        guard let prepared, canSave else {
            dismiss()
            return
        }
        // 保存成功后退出，失败时保留预览并允许再次保存。
        saving = true
        saveError = nil
        saveTask = Task { @MainActor in
            defer { saving = false }
            do {
                try await library.save(bundle: bundle, prepared: prepared)
                try Task.checkCancellation()
                guard ownerGeneration == auth.ownerGeneration else { return }
                dismiss()
            } catch {
                guard !Task.isCancelled, !SectionRequestFailure.isCancellation(error) else {
                    return
                }
                saveError = SectionRequestFailure(error: error).message
            }
        }
    }
}
