import Foundation
import SwiftUI

private enum HostPage: Hashable { case overview, installed }

extension AppThemePalette {
    fileprivate static let hostSuccess = Color(uiColor: .systemGreen)
}

struct HostListView: View {
    @EnvironmentObject private var authStore: AuthStore
    @EnvironmentObject private var dataStore: XboxDataStore
    @EnvironmentObject private var streamingStore: StreamingFeatureStore

    @State private var page: HostPage = .overview
    @State private var selectedHostID: String?
    let isActive: Bool

    init(isActive: Bool = true) {
        self.isActive = isActive
    }

    private var activationID: String {
        "\(authStore.ownerGeneration):\(isActive)"
    }

    var body: some View {
        NavigationStack(
            path: Binding(
                get: { page == .overview ? [] : [page] },
                set: { page = $0.last ?? .overview }
            )
        ) {
            content
                .appThemeCanvas()
                .appInformationTypography()
                .appPageToolbar("主机", showsBrand: true)
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button {
                            Task { await dataStore.refreshHosts() }
                        } label: {
                            AppIconView(.refresh, size: 22)
                                .foregroundStyle(AppThemePalette.textPrimary)
                        }
                        .accessibilityLabel("刷新主机")
                        .disabled(!authStore.isSignedIn || dataStore.isRefreshingHosts)
                    }
                    .sharedBackgroundVisibility(.hidden)
                }
                .navigationDestination(for: HostPage.self) { _ in
                    if let host = selectedHost { HostInstalledGamesView(host: host) }
                }
        }
        .onChange(of: authStore.ownerGeneration) { _, _ in
            page = .overview
            selectedHostID = nil
        }
        .task(id: activationID) {
            guard isActive, authStore.phase == .signedIn else { return }
            await dataStore.sync(
                session: authStore.session,
                ownerGeneration: authStore.ownerGeneration
            )
            await dataStore.activateHostsOnce()
        }
        .onChange(of: dataStore.hosts.map(\.id), initial: true) { _, _ in
            normalizeSelectedHost()
        }
    }

    @ViewBuilder
    private var content: some View {
        if !authStore.isSignedIn {
            XboxLoginView(
                isBusy: authStore.isBusy,
                errorMessage: authStore.errorMessage
            ) {
                Task { await authStore.retry() }
            }
        } else if dataStore.hosts.isEmpty {
            emptyContent
        } else {
            hostContent
        }
    }

    private var hostContent: some View {
        GeometryReader { geometry in
            AppPageScroll {
                VStack(alignment: .leading, spacing: AppLayoutPolicy.sectionSpacing) {
                    AppAccountRequestNotice()
                    hostRequestState

                    if let selectedHost {
                        HostOverviewHero(
                            host: selectedHost,
                            width: max(
                                0,
                                min(geometry.size.width, AppLayoutPolicy.contentMaximumWidth)
                                    - AppLayoutPolicy.pageInset * 2),
                            canStream: canStream(selectedHost),
                            powerAction: powerAction(for: selectedHost),
                            isPowerEnabled: powerAction(for: selectedHost).map {
                                isPowerActionEnabled($0, for: selectedHost)
                            } ?? false,
                            onStream: { streamSelectedHost(selectedHost) },
                            onPower: {
                                switch powerAction(for: selectedHost) {
                                case .powerOn: powerOnSelectedHost(selectedHost)
                                case .powerOff: powerOffSelectedHost(selectedHost)
                                case nil: break
                                }
                            },
                            onInstalled: { page = .installed }
                        )
                    }

                    if let error = dataStore.hostPowerCommandState.errorMessage {
                        Text(error).appFont(.caption).foregroundStyle(.orange)
                            .fixedSize(horizontal: false, vertical: true)
                    }

                    if dataStore.hosts.count > 1 { hostSwitcher }
                }
                .padding(AppLayoutPolicy.pageInset)
                .padding(.bottom, AppLayoutPolicy.pageInset)
                .frame(maxWidth: AppLayoutPolicy.contentMaximumWidth)
                .frame(maxWidth: .infinity)
            }
            .accessibilityIdentifier("host-overview-scroll")
            .scrollIndicators(.hidden)
            .refreshable { await dataStore.refreshHosts() }
        }
    }

    private var hostSwitcher: some View {
        ScrollViewReader { proxy in
            ScrollView(.horizontal) {
                HStack(spacing: AppLayoutPolicy.itemSpacing) {
                    ForEach(dataStore.hosts) { host in
                        Button {
                            selectedHostID = host.id
                        } label: {
                            HostSwitcherItem(host: host, isSelected: selectedHostID == host.id)
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("host-select-\(host.id)")
                        .accessibilityAddTraits(selectedHostID == host.id ? .isSelected : [])
                        .id(host.id)
                        .contextMenu {
                            if let action = powerAction(for: host) {
                                Button(
                                    LocalizedStringKey(action.title),
                                    role: action == .powerOff ? .destructive : nil
                                ) {
                                    switch action {
                                    case .powerOn: powerOnSelectedHost(host)
                                    case .powerOff: powerOffSelectedHost(host)
                                    }
                                }
                                .disabled(!isPowerActionEnabled(action, for: host))
                            }
                        }
                    }
                }
                .scrollTargetLayout()
            }
            .scrollIndicators(.hidden)
            .accessibilityIdentifier("host-switcher")
            .onChange(of: selectedHostID, initial: true) { _, id in
                if let id { proxy.scrollTo(id, anchor: .center) }
            }
        }
    }

    private var selectedHost: XboxHostSummary? {
        dataStore.hosts.first { $0.id == selectedHostID } ?? dataStore.hosts.first
    }

    private func normalizeSelectedHost() {
        guard let firstHost = dataStore.hosts.first else {
            selectedHostID = nil
            page = .overview
            return
        }
        if !dataStore.hosts.contains(where: { $0.id == selectedHostID }) {
            selectedHostID = firstHost.id
        }
    }

    private func canPowerOn(_ host: XboxHostSummary) -> Bool {
        return host.commandID != nil
            && host.remoteManagementEnabled != false
            && !dataStore.hostPowerCommandState.isExecuting
    }

    private func canPowerOff(_ host: XboxHostSummary) -> Bool {
        return host.commandID != nil
            && !dataStore.hostPowerCommandState.isExecuting
    }

    private func powerAction(for host: XboxHostSummary) -> HostPowerAction? {
        switch host.powerState {
        case "On":
            return host.commandID != nil ? .powerOff : nil
        case "Off", "ConnectedStandby", "Connected":
            return host.commandID != nil && host.remoteManagementEnabled != false
                ? .powerOn
                : nil
        default:
            return nil
        }
    }

    private func isPowerActionEnabled(
        _ action: HostPowerAction,
        for host: XboxHostSummary
    ) -> Bool {
        switch action {
        case .powerOn: canPowerOn(host)
        case .powerOff: canPowerOff(host)
        }
    }

    private func canStream(_ host: XboxHostSummary) -> Bool {
        host.canStartRemotePlay
            && !dataStore.hostPowerCommandState.isExecuting
    }

    private func powerOnSelectedHost(_ host: XboxHostSummary) {
        Task { await dataStore.powerOn(host: host) }
    }

    private func powerOffSelectedHost(_ host: XboxHostSummary) {
        Task { await dataStore.powerOff(host: host) }
    }

    private func streamSelectedHost(_ host: XboxHostSummary) {
        guard let targetID = host.streamTargetID else { return }
        streamingStore.startHome(targetID: targetID) { startup in
            try await authStore.prepareHomeAccess(startup: startup)
        }
    }

    @ViewBuilder
    private var emptyContent: some View {
        switch dataStore.hostPhase {
        case .idle, .loading:
            HostListLoadingView()
                .refreshable { await dataStore.refreshHosts() }
        case .failed:
            refreshableEmptyState { hostRequestState.padding(AppLayoutPolicy.pageInset) }
        case .loaded:
            refreshableEmptyState {
                VStack(spacing: 16) {
                    hostRequestState
                    AppThemeEmptyState(
                        title: "暂无可用主机",
                        systemImage: "desktopcomputer",
                        description: "请确认主机已绑定到当前 Xbox 账户",
                        actionTitle: "刷新"
                    ) {
                        Task { await dataStore.refreshHosts() }
                    }
                }
            }
        }
    }

    private var hostRequestState: some View {
        AppSectionRequestState(
            title: "主机列表",
            status: SectionRequestStatus(
                phase: dataStore.hostPhase, isBusy: dataStore.isRefreshingHosts,
                hasSnapshot: dataStore.hostsUpdatedAt != nil, updatedAt: dataStore.hostsUpdatedAt,
                attempt: dataStore.hostsUpdatedAt == nil ? .initial : .refresh,
                failure: dataStore.hostFailure), identifier: "hosts"
        ) {
            await dataStore.refreshHosts(reason: .manualRetry)
        }
    }

    private func refreshableEmptyState<Content: View>(
        @ViewBuilder content: @escaping () -> Content
    ) -> some View {
        GeometryReader { geometry in
            AppPageScroll {
                content()
                    .frame(maxWidth: .infinity, minHeight: geometry.size.height)
            }
            .refreshable { await dataStore.refreshHosts() }
        }
    }
}

extension XboxHostSummary {
    fileprivate var consoleAssetName: String {
        consoleType.lowercased().replacingOccurrences(of: " ", with: "").contains("seriess")
            ? "ConsoleSeriesS" : "ConsoleSeriesX"
    }

    fileprivate var modelTitle: String {
        switch consoleType.lowercased().replacingOccurrences(of: " ", with: "") {
        case "xboxseriesx": "Xbox Series X"
        case "xboxseriess": "Xbox Series S"
        case "xboxonex": "Xbox One X"
        case "xboxones": "Xbox One S"
        case "xboxone": "Xbox One"
        default: consoleType
        }
    }

    fileprivate var statusColor: Color {
        switch powerState {
        case "On": AppThemePalette.hostSuccess
        case "ConnectedStandby", "Connected": .orange
        default: AppThemePalette.textSecondary
        }
    }
}

/// 主图、设备事实和动作共用一个身份区，机器选择紧接在下方。
private struct HostOverviewHero: View {
    @Environment(\.locale) private var locale
    let host: XboxHostSummary
    let width: CGFloat
    let canStream: Bool
    let powerAction: HostPowerAction?
    let isPowerEnabled: Bool
    let onStream: () -> Void
    let onPower: () -> Void
    let onInstalled: () -> Void

    var body: some View {
        let imageWidth = min(440, width * 0.42)
        VStack(alignment: .leading, spacing: AppLayoutPolicy.groupSpacing) {
            HStack(spacing: AppLayoutPolicy.pageInset) {
                artwork(width: imageWidth, height: min(360, imageWidth / 0.7))
                VStack(alignment: .leading, spacing: AppLayoutPolicy.groupSpacing) {
                    details
                    if width >= 480 { actions(expands: false) }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            if width < 480 { actions(expands: true) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("host-selected-hero")
    }

    private func artwork(width: CGFloat, height: CGFloat) -> some View {
        // 原图四周为透明留白，按主体比例裁切使主机完整占据左侧空间。
        Image(host.consoleAssetName)
            .resizable().scaledToFill()
            .frame(width: width, height: height)
            .clipped()
            .shadow(color: .black.opacity(0.16), radius: 18, y: 12)
            .accessibilityHidden(true)
    }

    private var details: some View {
        VStack(alignment: .leading, spacing: AppLayoutPolicy.groupSpacing) {
            VStack(alignment: .leading, spacing: AppLayoutPolicy.detailSpacing) {
                Text(host.name).appFont(.title, weight: .bold)
                    .foregroundStyle(AppThemePalette.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("host-selected-name")
                Text(host.modelTitle).appFont(.bodySmall)
                    .foregroundStyle(AppThemePalette.textSecondary)
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: AppLayoutPolicy.groupSpacing) {
                        status
                        storage
                    }
                    VStack(alignment: .leading, spacing: AppLayoutPolicy.detailSpacing) {
                        status
                        storage
                    }
                }
            }

            VStack(alignment: .leading, spacing: AppLayoutPolicy.detailSpacing) {
                management
                // 常态串流能力由游玩按钮表达，关闭和同步状态保留明确说明。
                if host.consoleStreamingEnabled != true {
                    streaming
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func actions(expands: Bool) -> some View {
        HStack(alignment: .top, spacing: 20) {
            HostActionButton(
                title: "开始游玩", systemImage: "play",
                isEnabled: canStream, action: onStream,
                accessibilityLabel: appLocalizedFormat("开始游玩 %@", locale: locale, host.name),
                accessibilityHint: appLocalizedString(
                    host.readinessDescription, locale: locale),
                identifier: "host-start-stream"
            )
            .frame(maxWidth: expands ? .infinity : nil)
            if let powerAction {
                HostActionButton(
                    title: powerAction.title, systemImage: "power",
                    isEnabled: isPowerEnabled, action: onPower,
                    accessibilityLabel: appLocalizedFormat(
                        powerAction == .powerOn ? "开机 %@" : "关机 %@", locale: locale, host.name),
                    accessibilityHint: appLocalizedString(
                        powerAction == .powerOn ? "启动当前主机" : "关闭当前主机", locale: locale),
                    identifier: "host-power"
                )
                .frame(maxWidth: expands ? .infinity : nil)
            }
            HostActionButton(
                title: "安装游戏", systemImage: "books.vertical",
                isEnabled: true, action: onInstalled,
                accessibilityLabel: appLocalizedString("安装游戏", locale: locale),
                accessibilityHint: appLocalizedString("查看主机上的游戏与存储空间", locale: locale),
                identifier: "host-open-installed"
            )
            .frame(maxWidth: expands ? .infinity : nil)
        }
        .padding(.top, AppLayoutPolicy.itemSpacing)
    }

    private var status: some View {
        HStack(spacing: 5) {
            Circle().fill(host.statusColor).frame(width: 6, height: 6)
            Text(LocalizedStringKey(host.statusTitle)).appFont(.caption)
                .foregroundStyle(AppThemePalette.textSecondary)
        }.fixedSize()
    }

    @ViewBuilder
    private var storage: some View {
        let known = host.storageDevices.compactMap(\.freeBytes)
        if !known.isEmpty {
            let free = known.reduce(UInt64(0)) { total, bytes in
                total + min(bytes, UInt64.max - total)
            }
            let formatted = ByteCountFormatter.string(
                fromByteCount: Int64(clamping: free), countStyle: .file)
            Text(appLocalizedFormat("可用 %@", locale: locale, formatted))
                .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
                .fixedSize()
        } else if !host.storageDevices.isEmpty {
            Text(appLocalizedFormat("%lld 个存储设备", locale: locale, host.storageDevices.count))
                .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
                .fixedSize()
        }
    }

    private var management: some View {
        HostCapability(
            title: "远程管理", value: host.remoteManagementEnabled)
    }

    private var streaming: some View {
        HostCapability(
            title: "主机串流", value: host.consoleStreamingEnabled)
    }
}

private struct HostCapability: View {
    let title: String
    let value: Bool?

    var body: some View {
        (Text(LocalizedStringKey(title)) + Text(" · ") + Text(LocalizedStringKey(valueTitle)))
            .appFont(.caption)
            .foregroundStyle(AppThemePalette.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityElement(children: .combine)
    }

    private var valueTitle: String {
        switch value {
        case true: "已开启"
        case false: "已关闭"
        case nil: "同步中"
        }
    }

}

private struct HostSwitcherItem: View {
    let host: XboxHostSummary
    let isSelected: Bool

    var body: some View {
        HStack(spacing: AppLayoutPolicy.itemSpacing) {
            Image(host.consoleAssetName).resizable().scaledToFit()
                .frame(width: 30, height: 40).accessibilityHidden(true)
            VStack(alignment: .leading, spacing: AppLayoutPolicy.detailSpacing) {
                Text(host.name).appFont(.bodySmall, weight: .medium)
                    .foregroundStyle(AppThemePalette.textPrimary).lineLimit(1)
                HStack(spacing: 5) {
                    Circle().fill(host.statusColor).frame(width: 5, height: 5)
                    Text(LocalizedStringKey(host.statusTitle)).appFont(.caption)
                        .foregroundStyle(AppThemePalette.textSecondary).lineLimit(1)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(AppLayoutPolicy.rowInset)
        .frame(width: 168, height: 64)
        .background(
            AppThemePalette.brand.opacity(isSelected ? 0.08 : 0.025),
            in: RoundedRectangle(cornerRadius: AppLayoutPolicy.cardRadius)
        )
        .overlay {
            RoundedRectangle(cornerRadius: AppLayoutPolicy.cardRadius)
                .strokeBorder(
                    isSelected ? AppThemePalette.brand : AppThemePalette.border, lineWidth: 1)
        }
        .accessibilityElement(children: .combine)
        .accessibilityHint("选择主机，长按显示电源操作")
    }
}

private enum HostPowerAction: Equatable {
    case powerOn
    case powerOff

    var title: String {
        switch self {
        case .powerOn: "开机"
        case .powerOff: "关机"
        }
    }

}

private struct HostActionButton: View {
    let title: String
    let systemImage: String
    let isEnabled: Bool
    let action: () -> Void
    let accessibilityLabel: String
    let accessibilityHint: String
    let identifier: String

    var body: some View {
        Button(action: action) {
            VStack(spacing: AppLayoutPolicy.detailSpacing) {
                Image(systemName: systemImage)
                    .font(.system(size: 18, weight: .regular))
                    .frame(width: 44, height: 44)
                Text(LocalizedStringKey(title)).appFont(.caption)
            }
            .foregroundStyle(AppThemePalette.textSecondary)
            .opacity(isEnabled ? 1 : 0.38)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!isEnabled)
        .accessibilityLabel(accessibilityLabel)
        .accessibilityHint(accessibilityHint)
        .accessibilityIdentifier(identifier)
    }
}

private struct HostListLoadingView: View {
    var body: some View {
        AppPageScroll {
            VStack(alignment: .leading, spacing: AppLayoutPolicy.groupSpacing) {
                RoundedRectangle(cornerRadius: AppLayoutPolicy.cardRadius)
                    .fill(.quaternary).frame(height: 240)
                Text("客厅 Xbox").appFont(.title, weight: .bold).redacted(reason: .placeholder)
                Text("Xbox Series X").appFont(.bodySmall).redacted(reason: .placeholder)
                HStack(spacing: 20) {
                    ForEach(0..<3, id: \.self) { _ in
                        Circle().fill(.quaternary).frame(width: 56, height: 56)
                    }
                }
                RoundedRectangle(cornerRadius: AppLayoutPolicy.cardRadius)
                    .fill(.quaternary).frame(width: 168, height: 64)
                    .padding(.top, AppLayoutPolicy.sectionSpacing)
            }
            .padding(AppLayoutPolicy.pageInset)
            .frame(maxWidth: AppLayoutPolicy.contentMaximumWidth)
            .frame(maxWidth: .infinity)
        }
        .scrollIndicators(.hidden)
        .skeletonPulse(accessibilityLabel: "正在载入主机")
    }
}
