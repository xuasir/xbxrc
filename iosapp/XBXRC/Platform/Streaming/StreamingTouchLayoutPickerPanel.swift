import SwiftUI

struct StreamingTouchLayoutPickerPanel: View {
    @ObservedObject var controller: StreamingTouchLayoutController
    let onClose: () -> Void
    @EnvironmentObject private var settings: AppSettingsStore
    @EnvironmentObject private var library: SavedTouchLayoutsStore
    @EnvironmentObject private var auth: AuthStore
    @EnvironmentObject private var cloud: CloudLibraryStore
    @Environment(\.locale) private var locale
    @Environment(\.displayScale) private var displayScale
    @State private var search = ""
    @State private var selectionTask: Task<Void, Never>?
    @State private var catalogTask: Task<Void, Never>?

    var body: some View {
        GeometryReader { geometry in
            // 横屏为列表保留操作空间，竖屏继续使用居中的半屏浮层。
            let size = CGSize(
                width: min(460, max(1, geometry.size.width - 28)),
                height: max(1, min(max(320, geometry.size.height * 0.5), geometry.size.height - 28))
            )
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
                        if controller.streamTarget == .cloud {
                            currentGameLayouts
                        } else {
                            ForEach(
                                settings.mobile.touchLayoutTemplates.filter { matches($0.name) }
                            ) {
                                template in
                                row(
                                    name: template.name, selection: .template(template.id),
                                    identifier: "template-\(template.id)")
                            }
                            if let error = library.errorMessage {
                                Text(verbatim: error).font(.caption).foregroundStyle(
                                    .white.opacity(0.7))
                                Button("重试") { Task { await library.reload() } }.frame(
                                    minHeight: 44)
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
                        }
                        if let error = controller.loadError {
                            Text(verbatim: error).font(.caption).foregroundStyle(
                                .white.opacity(0.7)
                            )
                            .accessibilityIdentifier("stream-touch-layout-error")
                        }
                    }.padding(12)
                }.accessibilityIdentifier("stream-touch-layout-scroll")
                Text(
                    LocalizedStringKey(
                        controller.streamTarget == .cloud
                            ? "点选后下载并切换当前游戏的布局" : "新会话使用触屏设置中的默认布局")
                )
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
        .task(id: controller.cloudGame?.productID) {
            if controller.streamTarget == .cloud {
                await queryCloudCatalog(refresh: false)
            } else {
                await library.reload()
            }
        }
        .onDisappear {
            catalogTask?.cancel()
            selectionTask?.cancel()
            controller.cancelPendingSelection()
        }
    }

    @ViewBuilder
    private var currentGameLayouts: some View {
        if let game = controller.cloudGame {
            Text(verbatim: game.name).font(.subheadline.weight(.semibold)).padding(.top, 8)
                .accessibilityIdentifier("stream-touch-current-game")
        }
        if controller.cloudCatalogLoading {
            ProgressView("正在查询当前游戏布局")
                .accessibilityIdentifier("stream-touch-catalog-loading")
        }
        if let failure = controller.cloudCatalogFailure {
            GameTouchRequestFailure(
                error: failure, title: "当前游戏布局", identifier: "stream-touch-catalog"
            ) { refreshCatalog() }
        }
        if let catalog = controller.cloudCatalog {
            ForEach(catalog.sources.filter { $0.failure != nil }, id: \.source.rawValue) { result in
                if let failure = result.failure {
                    GameTouchRequestFailure(
                        error: failure, title: result.source.title,
                        identifier: "stream-touch-source-\(result.source.rawValue)"
                    ) { refreshCatalog() }
                }
            }
            ForEach(catalog.bundles) { descriptor in
                let choices = controller.cloudLayoutChoices.filter { $0.bundleID == descriptor.id }
                if let failure = controller.cloudDownloadFailures[descriptor.id] {
                    GameTouchRequestFailure(
                        error: failure, title: descriptor.source.title,
                        identifier: "stream-touch-download-\(descriptor.id)"
                    ) { choose(.gameBundle(descriptor.id)) }
                }
                if choices.isEmpty {
                    row(
                        name: appLocalizedString(descriptor.source.title, locale: locale),
                        detail: appLocalizedFormat("版本 %@", locale: locale, descriptor.tabVersion),
                        selection: .gameBundle(descriptor.id),
                        identifier: "cloud-bundle-\(descriptor.id)")
                } else {
                    ForEach(
                        choices.filter {
                            matches($0.choice.name) || matches(descriptor.source.title)
                        }
                    ) { choice in
                        row(
                            name: choice.choice.name,
                            detail: appLocalizedString(choice.source.shortTitle, locale: locale)
                                + " · "
                                + appLocalizedFormat("版本 %@", locale: locale, choice.version),
                            selection: choice.selection, identifier: "cloud-layout-\(choice.id)"
                        )
                        if !choice.choice.issues.isEmpty {
                            Text(verbatim: choice.choice.issues.joined(separator: "\n"))
                                .font(.caption).foregroundStyle(.white.opacity(0.7))
                        }
                    }
                }
            }
            if catalog.bundles.isEmpty && catalog.sources.allSatisfy({ $0.failure == nil }) {
                Text("此游戏暂无可用的手柄布局")
                    .font(.caption).foregroundStyle(.white.opacity(0.7))
            }
            Button("重新查询") { refreshCatalog() }.frame(minHeight: 44)
                .accessibilityIdentifier("stream-touch-catalog-refresh")
        }
    }

    private func queryCloudCatalog(refresh: Bool) async {
        await controller.loadCloudCatalog(
            client: library.client, serverRegion: settings.mobile.cloudServerRegion,
            locale: locale.identifier.replacingOccurrences(of: "_", with: "-"),
            displayScale: Double(displayScale), refresh: refresh,
            prepareAccess: {
                try await cloud.withCloudAccess(
                    prepareAccess: { try await auth.prepareCloudAccess() }, operation: { $0 })
            })
    }

    private func refreshCatalog() {
        catalogTask?.cancel()
        catalogTask = Task { await queryCloudCatalog(refresh: true) }
    }

    private func choose(_ selection: TouchLayoutSelection) {
        selectionTask?.cancel()
        selectionTask = Task { @MainActor in
            if await controller.select(
                selection, library: library, preferences: settings.mobile), !Task.isCancelled
            {
                onClose()
            }
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
            choose(selection)
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
        catalogTask?.cancel()
        selectionTask?.cancel()
        controller.cancelPendingSelection()
        onClose()
    }
}
