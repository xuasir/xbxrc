import SwiftUI

struct SavedTouchLayoutsView: View {
    @EnvironmentObject private var library: SavedTouchLayoutsStore
    @EnvironmentObject private var streaming: StreamingFeatureStore
    @Environment(\.locale) private var locale
    @State private var pendingDelete: XboxSavedTouchLayout?
    @State private var errorMessage: String?

    var body: some View {
        AppSettingsForm {
            if let error = errorMessage ?? library.errorMessage {
                Section {
                    Text(verbatim: error).foregroundStyle(AppThemePalette.textSecondary)
                    AppSettingsActionButton("重试") { Task { await library.reload() } }
                }
            }
            if library.isLoading && library.layouts.isEmpty {
                Section { ProgressView("正在读取已保存布局") }
            } else if library.layouts.isEmpty {
                Section {
                    Text("在云游戏详情中保存官方或社区布局，随后可在 Home 串流中切换。")
                        .foregroundStyle(AppThemePalette.textSecondary)
                        .accessibilityIdentifier("saved-touch-layouts-empty")
                }
            } else {
                ForEach(library.groups) { group in
                    Section {
                        ForEach(group.layouts) { item in savedRow(item) }
                    } header: {
                        Text(verbatim: group.name)
                    }
                }
            }
        }
        .appThemeCanvas().appPageToolbar("已保存的游戏布局")
        .task { await library.reload() }
        .confirmationDialog(
            "删除此游戏布局？",
            isPresented: Binding(
                get: { pendingDelete != nil }, set: { if !$0 { pendingDelete = nil } }),
            titleVisibility: .visible
        ) {
            if let item = pendingDelete {
                Button("删除布局", role: .destructive) {
                    pendingDelete = nil
                    Task {
                        do {
                            try await library.delete(item, controller: streaming.touchLayout)
                        } catch { errorMessage = CloudLibraryDiagnostics.safeError(error) }
                    }
                }.accessibilityIdentifier("saved-touch-confirm-delete")
            }
            Button("取消", role: .cancel) { pendingDelete = nil }
        } message: {
            Text("正在使用的布局会切换为通用 Xbox。")
        }
    }

    private func savedRow(_ item: XboxSavedTouchLayout) -> some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(verbatim: item.layoutName).appFont(.body, weight: .semibold)
                        .accessibilityIdentifier("saved-touch-name-\(item.id)")
                    if item.isDefault {
                        Text("默认布局").appFont(.caption).foregroundStyle(AppThemePalette.brand)
                            .fixedSize()
                            .accessibilityIdentifier("saved-touch-default-\(item.id)")
                    }
                }
                Text(
                    verbatim: appLocalizedString(item.source.shortTitle, locale: locale) + " · "
                        + appLocalizedFormat("版本 %@", locale: locale, item.bundleVersion)
                        + " · "
                        + ByteCountFormatter.string(
                            fromByteCount: Int64(clamping: item.byteCount), countStyle: .file)
                )
                .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
            }
            Spacer(minLength: 8)
            Button("删除布局", systemImage: "trash", role: .destructive) { pendingDelete = item }
                .buttonStyle(.plain).labelStyle(.iconOnly).frame(minWidth: 44, minHeight: 44)
                .disabled(library.deletingIDs.contains(item.id))
                .accessibilityLabel(
                    Text(verbatim: appLocalizedFormat("删除 %@", locale: locale, item.layoutName))
                )
                .accessibilityIdentifier("saved-touch-delete-\(item.id)")
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("saved-touch-layout-\(item.id)")
    }
}
