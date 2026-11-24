import SwiftUI

struct HostInstalledGamesView: View {
    let host: XboxHostSummary
    @EnvironmentObject private var auth: AuthStore
    @EnvironmentObject private var data: XboxDataStore
    @EnvironmentObject private var settings: AppSettingsStore
    @EnvironmentObject private var streaming: StreamingFeatureStore
    @EnvironmentObject private var cloud: CloudLibraryStore
    @Environment(\.appPageIsActive) private var isActive
    @Environment(\.locale) private var locale
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @State private var search = ""
    @State private var storageID = "all"
    @State private var sort: InstalledGameSort = .recent
    @State private var selectedGame: XboxInstalledGameSummary?

    private var consoleID: String? { host.commandID }
    private var state: XboxInstalledLibraryState {
        consoleID.flatMap { data.installedLibraryStates[$0] } ?? XboxInstalledLibraryState()
    }
    private var games: [XboxInstalledGameSummary] {
        PersonalLibraryPresentation.installed(
            state.snapshot?.games ?? [], search: search, storageID: storageID, sort: sort)
    }
    private var storageDevices: [XboxHostStorageSummary] {
        if let devices = state.snapshot?.storage, !devices.isEmpty { return devices }
        if let devices = data.hosts.first(where: { $0.id == host.id })?.storageDevices,
            !devices.isEmpty
        {
            return devices
        }
        return host.storageDevices
    }

    var body: some View {
        AppPageScroll {
            VStack(spacing: 0) {
                AppAccountRequestNotice()
                hostOverview
                VStack(alignment: .leading, spacing: 12) {
                    AppContentSearch(text: $search, prompt: "搜索安装游戏")
                        .accessibilityIdentifier("installed-content-search")
                    AppContentToolbar {
                        sortPicker
                        storagePicker
                    }
                    HStack(spacing: 8) {
                        Text(libraryHint).appFont(.caption)
                            .foregroundStyle(AppThemePalette.textSecondary)
                            .lineLimit(1)
                    }
                    .accessibilityIdentifier("installed-library-hint")
                    AppSectionRequestState(
                        title: "安装游戏", status: sectionStatus(.games), identifier: "installed-games"
                    ) { await retry(.games) }
                    AppSectionRequestState(
                        title: "游戏封面", status: sectionStatus(.artwork),
                        identifier: "installed-artwork"
                    ) { await retry(.artwork) }
                    gameGrid
                    if games.isEmpty, state.sections[.games]?.phase == .loaded {
                        Text(
                            LocalizedStringKey(
                                state.snapshot?.games == nil
                                    ? "安装游戏暂时无法载入" : search.isEmpty ? "当前筛选下暂无安装游戏" : "没有匹配的游戏")
                        )
                        .appFont(.body).foregroundStyle(AppThemePalette.textSecondary)
                    }

                }
                .padding(AppLayoutPolicy.pageInset).padding(.bottom, AppLayoutPolicy.pageInset)
                .frame(maxWidth: AppLayoutPolicy.contentMaximumWidth, alignment: .leading)
                .frame(maxWidth: .infinity)
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("installed-library-content")
            }
        }
        .accessibilityIdentifier("app-information-scroll")
        .scrollDismissesKeyboard(.interactively)
        .appThemeCanvas()
        .appPageToolbar("安装游戏")
        .appInformationTypography()
        .task(
            id:
                "\(auth.ownerGeneration):\(host.id):\(settings.mobile.catalogMarket):\(settings.mobile.catalogLanguage):\(isActive)"
        ) {
            guard isActive else { return }
            async let installed: Void = load(refresh: false)
            async let history: Void = data.activateLibraryOnce()
            _ = await (installed, history)
        }
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    Task { await load(refresh: true) }
                } label: {
                    AppIconView(.refresh, size: 22).foregroundStyle(AppThemePalette.textPrimary)
                }.disabled(state.isRefreshing).accessibilityLabel("刷新安装游戏")
            }.sharedBackgroundVisibility(.hidden)
        }
        .onChange(of: host.id) { _, _ in
            storageID = "all"
            selectedGame = nil
        }
        .onChange(of: storageDevices.map(\.id)) { _, devices in
            if storageID != "all", !devices.contains(storageID) { storageID = "all" }
        }
        .appRefreshable { await load(refresh: true) }
        .navigationDestination(item: $selectedGame) { game in
            if let titleID = game.titleID {
                PersonalGameDetailView(
                    game: data.games.first { $0.titleID == titleID }
                        ?? GameSummary(
                            id: titleID, titleID: titleID, name: game.name,
                            artworkURL: artwork(for: game),
                            lastPlayedAt: game.lastPlayedAt, playtimeMinutes: nil,
                            achievementProgress: nil,
                            productID: game.productID),
                    installedGame: game, host: host)
            } else {
                let detailState =
                    game.productID.map { cloud.productDetailState($0) }
                    ?? XboxSingleDataState()
                let detail = detailState.value
                AppInformationPage {
                    Section {
                        HStack(alignment: .center, spacing: 12) {
                            Text(detail?.name ?? game.name).appFont(.title, weight: .bold)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            HostStreamButton(host: host, size: 44, action: startHost)
                                .accessibilityIdentifier("personal-game-start-stream")
                        }
                        if let productID = game.productID {
                            AppSectionRequestState(
                                title: "游戏资料", status: detailState.requestStatus,
                                identifier: "installed-game-catalog"
                            ) {
                                await cloud.loadProductDetail(productID: productID, refresh: true)
                            }
                        }
                        LabeledContent("大小", value: bytes(game.sizeBytes))
                        if let date = game.installedAt {
                            LabeledContent(
                                "安装时间",
                                value: date.formatted(
                                    Date.FormatStyle(date: .abbreviated, time: .omitted).locale(
                                        locale)))
                        }
                        if let date = game.lastPlayedAt {
                            LabeledContent(
                                "最近游玩",
                                value: date.formatted(
                                    Date.FormatStyle(date: .abbreviated, time: .omitted).locale(
                                        locale)))
                        }
                    } footer: {
                        Text("连接后进入主机界面，选择此游戏开始游玩。")
                    }
                    if let description = detail?.description, !description.isEmpty {
                        Section("游戏简介") {
                            Text(description).appFont(.body)
                        }
                    }
                }
                .appPageToolbar("游戏详情")
                .task(id: "\(auth.ownerGeneration):\(cloud.productScopeID):\(game.productID ?? "")")
                {
                    if let productID = game.productID {
                        await cloud.loadProductDetail(productID: productID)
                    }
                }
                .appRefreshable {
                    if let productID = game.productID {
                        await cloud.loadProductDetail(productID: productID, refresh: true)
                    }
                }
            }
        }
    }

    private var hostOverview: some View {
        VStack(alignment: .leading, spacing: 12) {
            if dynamicTypeSize.isAccessibilitySize {
                hostInformation
                HStack(alignment: .bottom, spacing: 16) {
                    consoleArtwork(width: 96, height: 104)
                    Spacer(minLength: 0)
                    startButton
                }
            } else {
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .bottom, spacing: 32) {
                        hostInformation.frame(width: 320)
                        Spacer(minLength: 0)
                        HStack(alignment: .bottom, spacing: 16) {
                            consoleArtwork(width: 224, height: 224)
                            startButton
                        }
                    }
                    HStack(alignment: .bottom, spacing: 12) {
                        hostInformation
                        Spacer(minLength: 0)
                        VStack(alignment: .trailing, spacing: 8) {
                            consoleArtwork(width: 144, height: 176)
                            startButton
                        }
                    }
                }
            }
            AppSectionRequestState(
                title: "主机状态", status: sectionStatus(.status), identifier: "installed-status"
            ) { await retry(.status) }
            AppSectionRequestState(
                title: "存储信息", status: sectionStatus(.storage), identifier: "installed-storage"
            ) { await retry(.storage) }
        }
        .padding(.horizontal, AppLayoutPolicy.pageInset)
        .padding(.top, AppLayoutPolicy.sectionSpacing).padding(.bottom, AppLayoutPolicy.pageInset)
        .frame(maxWidth: AppLayoutPolicy.contentMaximumWidth, alignment: .leading)
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("installed-upper-region")
    }

    private var hostInformation: some View {
        VStack(alignment: .leading, spacing: 16) {
            VStack(alignment: .leading, spacing: 8) {
                Text(host.name).appFont(.hero, weight: .semibold)
                Text(LocalizedStringKey(hostStatus)).appFont(.body)
                    .foregroundStyle(AppThemePalette.textSecondary)
            }
            storageOverview
        }
        .frame(maxWidth: 320, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("installed-host-hero")
    }

    private func consoleArtwork(width: CGFloat, height: CGFloat) -> some View {
        Image(
            host.consoleType.lowercased().contains("series s") ? "ConsoleSeriesS" : "ConsoleSeriesX"
        )
        .resizable().scaledToFit()
        .frame(width: width, height: height)
        .accessibilityLabel(host.consoleType)
        .accessibilityIdentifier("installed-console-artwork")
    }

    private var startButton: some View {
        HostStreamButton(host: host, action: startHost)
        .accessibilityIdentifier("installed-start-stream")
    }

    private var storageOverview: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("存储").appFont(.caption, weight: .semibold)
            if let overview = PersonalLibraryPresentation.storageOverview(storageDevices),
                let total = overview.totalBytes, let free = overview.freeBytes
            {
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .top, spacing: 12) {
                        capacityMetric("已用", value: bytes(total - free))
                        capacityMetric("可用", value: bytes(free))
                        capacityMetric("总计", value: bytes(total))
                    }.fixedSize(horizontal: true, vertical: false)
                    VStack(alignment: .leading, spacing: 8) {
                        capacityMetric("已用", value: bytes(total - free))
                        capacityMetric("可用", value: bytes(free))
                        capacityMetric("总计", value: bytes(total))
                    }
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("installed-storage-capacity")
            }
            ForEach(storageDevices) { device in
                VStack(alignment: .leading, spacing: 4) {
                    HStack {
                        Text(device.name).appFont(.caption, weight: .semibold)
                        Spacer(minLength: 8)
                        Text(storageText(device)).appFont(.caption)
                            .foregroundStyle(AppThemePalette.textSecondary)
                    }
                    if let fraction = device.usedFraction {
                        ProgressView(value: fraction).tint(AppThemePalette.brand)
                    }
                }
            }
            if storageDevices.isEmpty, state.sections[.storage]?.phase == .loaded,
                state.sections[.storage]?.failure == nil
            {
                Text("存储信息暂缺").appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("installed-storage-summary")
    }

    private func capacityMetric(_ title: String, value: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(value).appFont(.bodySmall, weight: .semibold).monospacedDigit()
                .fixedSize(horizontal: true, vertical: false)
            Text(LocalizedStringKey(title)).appFont(.caption)
                .foregroundStyle(AppThemePalette.textSecondary)
                .fixedSize(horizontal: true, vertical: false)
        }.frame(maxWidth: .infinity, alignment: .leading)
    }

    private func sectionStatus(_ source: XboxInstalledSource) -> SectionRequestStatus {
        var status = (state.sections[source] ?? XboxSingleDataState()).requestStatus
        if source == .storage, !storageDevices.isEmpty {
            status.hasSnapshot = true
            status.updatedAt = status.updatedAt ?? data.hostsUpdatedAt
        }
        return status
    }
    private func retry(_ source: XboxInstalledSource) async {
        guard let consoleID else { return }
        await data.loadInstalledSection(
            consoleID: consoleID, market: settings.mobile.catalogMarket,
            language: settings.mobile.catalogLanguage, source: source)
    }

    private var libraryHint: String {
        appLocalizedFormat("安装游戏 · %lld", locale: locale, games.count)
    }

    private var gameGrid: some View {
        AppMediaGrid {
            ForEach(games) { game in
                Button {
                    selectedGame = game
                } label: {
                    VStack(alignment: .leading, spacing: AppLayoutPolicy.itemSpacing) {
                        Color.clear.aspectRatio(1, contentMode: .fit)
                            .overlay {
                                AsyncImage(url: artwork(for: game)) { image in
                                    image.resizable().scaledToFill()
                                } placeholder: {
                                    Rectangle().fill(.quaternary).overlay {
                                        AppIconView(.controller, size: 40)
                                    }
                                }
                            }
                            .clipShape(RoundedRectangle(cornerRadius: 8))
                            .accessibilityHidden(true)
                        Text(game.name).appFont(.body, weight: .semibold).foregroundStyle(
                            .primary)
                        Text(bytes(game.sizeBytes)).appFont(.caption).foregroundStyle(
                            .secondary)
                        if let storage = storageDevices.first(where: {
                            $0.id == game.storageDeviceID
                        }) {
                            Text(storage.name).appFont(.caption).foregroundStyle(
                                AppThemePalette.textSecondary)
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading).contentShape(
                        Rectangle())
                }
                .buttonStyle(.plain).accessibilityIdentifier("installed-game-\(game.id)")
            }
        }
    }

    private func load(refresh: Bool) async {
        guard let consoleID else { return }
        await data.loadInstalledLibrary(
            consoleID: consoleID, market: settings.mobile.catalogMarket,
            language: settings.mobile.catalogLanguage,
            refresh: refresh)
    }
    private func bytes(_ count: UInt64?) -> String {
        count.map {
            ByteCountFormatter.string(fromByteCount: Int64(clamping: $0), countStyle: .file)
        } ?? appLocalizedString("大小未知", locale: locale)
    }
    private func storageText(_ device: XboxHostStorageSummary) -> String {
        guard let free = device.freeBytes else {
            return appLocalizedString("空间同步中", locale: locale)
        }
        return String(
            format: appLocalizedString("可用 %@ / %@", locale: locale),
            bytes(device.totalBytes.map { min(free, $0) } ?? free),
            bytes(device.totalBytes))
    }
    private var sortPicker: some View {
        AppInformationPicker("游戏排序", selection: $sort, selectedTitle: sort.title) {
            ForEach(InstalledGameSort.allCases) { Text(LocalizedStringKey($0.title)).tag($0) }
        }
    }

    @ViewBuilder
    private var storagePicker: some View {
        if !storageDevices.isEmpty {
            AppInformationPicker(
                "存储设备", selection: $storageID,
                selectedTitle: storageDevices.first { $0.id == storageID }?.name ?? "全部设备"
            ) {
                Text("全部设备").tag("all")
                ForEach(storageDevices) { Text($0.name).tag($0.id) }
            }
        }
    }

    private var hostStatus: String {
        switch state.snapshot?.powerState ?? host.powerState {
        case "On": "已开机"
        case "Connected", "ConnectedStandby": "待机"
        case "Off": "已关机"
        default: "状态同步中"
        }
    }
    private func artwork(for game: XboxInstalledGameSummary) -> URL? {
        game.artworkURL ?? data.games.first { $0.titleID == game.titleID }?.artworkURL
    }
    private func startHost() {
        if let target = host.streamTargetID {
            streaming.startHome(targetID: target) { startup in
                try await auth.prepareHomeAccess(startup: startup)
            }
        }
    }
}

/// 主机概览与游戏标题共用圆形串流入口和可用状态。
struct HostStreamButton: View {
    let host: XboxHostSummary
    var size: CGFloat = 56
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: "play.fill")
                .font(.system(size: size >= 56 ? 20 : 16, weight: .semibold))
                .offset(x: 1.5).frame(width: size, height: size)
                .foregroundStyle(.white)
                .background(AppThemePalette.brand, in: Circle())
        }
        .buttonStyle(.plain).disabled(!host.canStartRemotePlay)
        .opacity(host.canStartRemotePlay ? 1 : 0.45)
        .accessibilityLabel("串流此主机")
        .accessibilityHint("连接后进入主机界面，选择此游戏开始游玩。")
    }
}
