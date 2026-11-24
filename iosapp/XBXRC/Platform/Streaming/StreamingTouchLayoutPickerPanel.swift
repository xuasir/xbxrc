import SwiftUI

struct StreamingTouchLayoutPickerPanel: View {
    @ObservedObject var controller: StreamingTouchLayoutController
    let onClose: () -> Void
    @EnvironmentObject private var settings: AppSettingsStore
    @EnvironmentObject private var library: SavedTouchLayoutsStore
    @Environment(\.locale) private var locale
    @State private var search = ""
    @State private var selectionTask: Task<Void, Never>?

    var body: some View {
        GeometryReader { geometry in
            let size = CGSize(
                width: min(460, max(1, geometry.size.width - 28)),
                height: max(1, min(geometry.size.height * 0.5, geometry.size.height - 28)))
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    Text("切换触屏布局").font(.headline)
                    Spacer(minLength: 8)
                    Button(action: close) {
                        Image(systemName: "xmark")
                            .frame(width: 44, height: 44).contentShape(Rectangle())
                    }
                    .buttonStyle(.plain).accessibilityLabel("关闭布局选择")
                    .accessibilityIdentifier("stream-touch-layout-close")
                }.padding(.leading, 16).padding(.trailing, 4)
                HStack(spacing: 8) {
                    Image(systemName: "magnifyingglass").accessibilityHidden(true)
                    TextField("搜索游戏或布局", text: $search).submitLabel(.search)
                        .accessibilityIdentifier("stream-touch-layout-search")
                }
                .padding(12).background(
                    .white.opacity(0.08), in: RoundedRectangle(cornerRadius: 10)
                )
                .padding(.horizontal, 12).padding(.bottom, 8)
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 8) {
                        if matches("通用 Xbox") {
                            row(
                                name: appLocalizedString("通用 Xbox", locale: locale),
                                selection: .standard, identifier: "standard")
                        }
                        ForEach(settings.mobile.touchLayoutTemplates.filter { matches($0.name) }) {
                            template in
                            row(
                                name: template.name, selection: .template(template.id),
                                identifier: "template-\(template.id)")
                        }
                        if let error = library.errorMessage {
                            Text(verbatim: error).font(.caption).foregroundStyle(
                                .white.opacity(0.7))
                            Button("重试") { Task { await library.reload() } }.frame(minHeight: 44)
                        }
                        if library.isLoading && library.layouts.isEmpty {
                            ProgressView("正在读取已保存布局")
                        }
                        ForEach(library.groups) { group in
                            let items = group.layouts.filter {
                                matches(group.name) || matches($0.layoutName)
                            }
                            if !items.isEmpty {
                                Text(verbatim: group.name).font(.subheadline.weight(.semibold))
                                    .padding(.top, 12)
                                ForEach(items) { item in
                                    row(
                                        name: item.layoutName,
                                        detail: appLocalizedFormat(
                                            "版本 %@", locale: locale, item.bundleVersion),
                                        selection: .saved(item.id), identifier: item.id
                                    )
                                    .disabled(library.deletingIDs.contains(item.id))
                                }
                            }
                        }
                        if library.layouts.isEmpty && !library.isLoading {
                            Text("在云游戏详情中保存官方或社区布局，随后可在 Home 串流中切换。")
                                .font(.caption).foregroundStyle(.white.opacity(0.7)).padding(
                                    .top, 12)
                        }
                        if let error = controller.loadError {
                            Text(verbatim: error).font(.caption).foregroundStyle(
                                .white.opacity(0.7)
                            )
                            .accessibilityIdentifier("stream-touch-layout-error")
                        }
                    }.padding(12)
                }
                Text("新会话使用触屏设置中的默认布局")
                    .font(.caption).foregroundStyle(.white.opacity(0.7)).padding(12)
            }
            .foregroundStyle(.white).tint(.white)
            .environment(\.colorScheme, .dark)
            .frame(width: size.width, height: min(580, size.height))
            .appSurface(radius: 14, prominent: true, forceDark: true)
            .clipShape(RoundedRectangle(cornerRadius: 14))
            .position(x: geometry.size.width / 2, y: geometry.size.height / 2)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("stream-touch-layout-panel")
        }
        .task { await library.reload() }
        .onDisappear {
            selectionTask?.cancel()
            controller.cancelPendingSelection()
        }
    }

    private func matches(_ text: String) -> Bool {
        search.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || text.localizedCaseInsensitiveContains(
                search.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    private func row(
        name: String, detail: String? = nil, selection: TouchLayoutSelection, identifier: String
    ) -> some View {
        let selected = controller.effectiveSelection(preferences: settings.mobile) == selection
        return Button {
            selectionTask?.cancel()
            selectionTask = Task { @MainActor in
                if await controller.select(
                    selection, library: library, preferences: settings.mobile), !Task.isCancelled
                {
                    onClose()
                }
            }
        } label: {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(verbatim: name).font(.subheadline.weight(.medium))
                    if let detail {
                        Text(verbatim: detail).font(.caption).foregroundStyle(.white.opacity(0.7))
                    }
                }
                Spacer(minLength: 8)
                if controller.pendingSelection == selection {
                    ProgressView()
                } else if selected {
                    Image(systemName: "checkmark.circle.fill").foregroundStyle(
                        AppThemePalette.brand)
                }
            }
            .padding(12).frame(minHeight: 44)
            .background(
                selected ? .white.opacity(0.12) : .white.opacity(0.04),
                in: RoundedRectangle(cornerRadius: 10)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityValue(Text(LocalizedStringKey(selected ? "已选中" : "")))
        .accessibilityIdentifier("stream-touch-layout-choice-\(identifier)")
    }

    private func close() {
        selectionTask?.cancel()
        controller.cancelPendingSelection()
        onClose()
    }
}
