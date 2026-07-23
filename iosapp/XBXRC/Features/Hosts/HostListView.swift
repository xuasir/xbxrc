import Foundation
import SwiftUI

private extension AppThemePalette {
    static let hostControlForeground = Color(uiColor: .label)
    static let hostDanger = Color(uiColor: .systemRed)
    static let hostSuccess = Color(uiColor: .systemGreen)
}

struct HostListView: View {
    @EnvironmentObject private var authStore: AuthStore
    @EnvironmentObject private var dataStore: XboxDataStore
    @EnvironmentObject private var streamingStore: StreamingFeatureStore

    @State private var selectedHostID: String?
    @State private var actionMenuHost: XboxHostSummary?
    let isActive: Bool

    init(isActive: Bool = true) {
        self.isActive = isActive
    }

    private var activationID: String {
        "\(authStore.ownerGeneration):\(isActive)"
    }

    var body: some View {
        NavigationStack {
            content
                .appThemeCanvas()
                .navigationTitle("")
                .toolbar(.hidden, for: .navigationBar)
        }
        .task(id: activationID) {
            guard isActive, authStore.phase == .signedIn else { return }
            await dataStore.sync(
                session: authStore.session,
                ownerGeneration: authStore.ownerGeneration
            )
            await dataStore.activateHostsOnce()
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
            ScrollView {
                VStack(spacing: 0) {
                    if let errorMessage = dataStore.hostErrorMessage {
                        Text(errorMessage)
                            .font(.subheadline)
                            .foregroundStyle(.orange)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 24)
                            .padding(.bottom, 12)
                    }

                    OrbitCardCarousel(
                        items: dataStore.hosts,
                        selection: $selectedHostID,
                        cardWidth: max(0, geometry.size.width - 32),
                        showsCardChrome: false
                    ) { host in
                        HostCarouselCard(host: host)
                            .onLongPressGesture(minimumDuration: 0.45, maximumDistance: 12) {
                                actionMenuHost = host
                            }
                            .accessibilityAction(named: "主机电源操作") {
                                actionMenuHost = host
                            }
                            .confirmationDialog(
                                host.name,
                                isPresented: actionMenuBinding(for: host),
                                titleVisibility: .visible
                            ) {
                                switch powerAction(for: host) {
                                case .powerOn:
                                    Button("开机", systemImage: "power") {
                                        powerOnSelectedHost(host)
                                    }
                                    .disabled(!isPowerActionEnabled(.powerOn, for: host))
                                case .powerOff:
                                    Button("关机", role: .destructive) {
                                        powerOffSelectedHost(host)
                                    }
                                    .disabled(!isPowerActionEnabled(.powerOff, for: host))
                                case nil:
                                    EmptyView()
                                }

                                Button("取消", role: .cancel) {}
                            } message: {
                                Text("选择当前主机的电源操作")
                            }
                    }
                    .frame(height: 390)
                    .padding(.top, 12)
                    .padding(.bottom, dataStore.hosts.count > 1 ? 10 : 20)

                    if dataStore.hosts.count > 1 {
                        HostPageIndicator(
                            count: dataStore.hosts.count,
                            selectedIndex: selectedHostIndex
                        )
                        .padding(.bottom, 14)
                    }

                    if let selectedHost {
                        HostInfoRows(host: selectedHost)
                            .id(selectedHost.id)
                            .padding(.horizontal, 16)
                            .padding(.top, dataStore.hosts.count > 1 ? 0 : 10)
                            .transition(.opacity.combined(with: .move(edge: .bottom)))
                            .animation(.easeInOut(duration: 0.22), value: selectedHostID)
                    }

                    if let powerCommandError = dataStore.hostPowerCommandState.errorMessage {
                        Text(powerCommandError)
                            .font(.caption)
                            .foregroundStyle(.orange)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 24)
                    }

                    if let selectedHost {
                        Spacer(minLength: 24)

                        HostActionBar(
                            host: selectedHost,
                            canStream: canStream(selectedHost),
                            powerAction: powerAction(for: selectedHost),
                            isPowerEnabled: powerAction(for: selectedHost).map {
                                isPowerActionEnabled($0, for: selectedHost)
                            } ?? false,
                            onStream: { streamSelectedHost(selectedHost) },
                            onPowerOn: { powerOnSelectedHost(selectedHost) },
                            onPowerOff: { powerOffSelectedHost(selectedHost) }
                        )

                        Spacer(minLength: 24)
                    }
                }
                .frame(maxWidth: .infinity, minHeight: geometry.size.height)
            }
            .scrollIndicators(.hidden)
            .safeAreaPadding(.bottom, 18)
            .refreshable { await dataStore.refreshHosts() }
        }
        .task(id: dataStore.hosts.map(\.id)) {
            normalizeSelectedHost()
        }
    }

    private var selectedHost: XboxHostSummary? {
        dataStore.hosts.first { $0.id == selectedHostID }
            ?? dataStore.hosts.first
    }

    private var selectedHostIndex: Int {
        dataStore.hosts.firstIndex { $0.id == selectedHostID } ?? 0
    }

    private func actionMenuBinding(for host: XboxHostSummary) -> Binding<Bool> {
        Binding(
            get: { actionMenuHost?.id == host.id },
            set: { isPresented in
                if !isPresented, actionMenuHost?.id == host.id {
                    actionMenuHost = nil
                }
            }
        )
    }

    private func normalizeSelectedHost() {
        guard let firstHost = dataStore.hosts.first else {
            selectedHostID = nil
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
        actionMenuHost = nil
        Task { await dataStore.powerOn(host: host) }
    }

    private func powerOffSelectedHost(_ host: XboxHostSummary) {
        actionMenuHost = nil
        Task { await dataStore.powerOff(host: host) }
    }

    private func streamSelectedHost(_ host: XboxHostSummary) {
        guard let targetID = host.streamTargetID else { return }
        streamingStore.startHome(targetID: targetID) {
            try await authStore.prepareHomeAccess()
        }
    }

    @ViewBuilder
    private var emptyContent: some View {
        switch dataStore.hostPhase {
        case .idle, .loading:
            HostListLoadingView()
                .refreshable { await dataStore.refreshHosts() }
        case .failed:
            refreshableEmptyState {
                AppThemeEmptyState(
                    title: "无法载入主机",
                    systemImage: "exclamationmark.triangle",
                    description: dataStore.hostErrorMessage ?? "Xbox 服务暂时不可用",
                    actionTitle: "重新加载"
                ) {
                    Task { await dataStore.refreshHosts(reason: .manualRetry) }
                }
            }
        case .loaded:
            refreshableEmptyState {
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

    private func refreshableEmptyState<Content: View>(
        @ViewBuilder content: @escaping () -> Content
    ) -> some View {
        GeometryReader { geometry in
            ScrollView {
                content()
                    .frame(maxWidth: .infinity, minHeight: geometry.size.height)
            }
            .refreshable { await dataStore.refreshHosts() }
        }
    }
}

private struct HostCarouselCard: View {
    let host: XboxHostSummary

    var body: some View {
        VStack(spacing: 0) {
            Image(consoleAssetName)
                .resizable()
                .scaledToFit()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .padding(.horizontal, 18)
                .padding(.vertical, 12)
                .brightness(0.08)
                .contrast(1.07)
                .scaleEffect(consoleImageScale)
                .offset(y: consoleImageOffset)
                .shadow(color: .black.opacity(0.24), radius: 16, y: 10)
                .clipped()
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(host.name)
                        .font(.title3.weight(.bold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                        .minimumScaleFactor(0.68)
                        .layoutPriority(1)

                    Spacer(minLength: 4)

                    HStack(spacing: 5) {
                        Circle()
                            .fill(statusColor)
                            .frame(width: 7, height: 7)

                        Text(host.statusTitle)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    .font(.caption.weight(.semibold))
                    .fixedSize(horizontal: true, vertical: false)
                }

                if let storageDescription {
                    Label(storageDescription, systemImage: "internaldrive.fill")
                        .font(.subheadline.weight(.medium))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .minimumScaleFactor(0.72)
                }
            }
            .frame(maxWidth: .infinity, minHeight: 54, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.vertical, 5)
        }
        .compositingGroup()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityLabel)
        .accessibilityHint("长按显示主机电源操作")
    }

    private var consoleAssetName: String {
        host.consoleType.lowercased().contains("series s")
            ? "ConsoleSeriesS"
            : "ConsoleSeriesX"
    }

    private var consoleImageScale: CGFloat {
        host.consoleType.lowercased().contains("series s") ? 1.16 : 1.38
    }

    private var consoleImageOffset: CGFloat {
        host.consoleType.lowercased().contains("series s") ? 2 : 8
    }

    private var statusColor: Color {
        switch host.powerState {
        case "On": .green
        case "ConnectedStandby", "Connected": .orange
        default: .secondary
        }
    }

    private var storageDescription: String? {
        guard !host.storageDevices.isEmpty else { return nil }

        let knownDevices = host.storageDevices.compactMap { storage -> UInt64? in
            storage.freeBytes
        }
        guard !knownDevices.isEmpty else {
            return "\(host.storageDevices.count) 个存储设备"
        }

        let freeBytes = knownDevices.reduce(UInt64(0), +)
        return "可用 \(formattedByteCount(freeBytes))"
    }

    private func formattedByteCount(_ value: UInt64) -> String {
        ByteCountFormatter.string(
            fromByteCount: Int64(min(value, UInt64(Int64.max))),
            countStyle: .file
        )
    }

    private var accessibilityLabel: String {
        [
            host.name,
            host.statusTitle,
            storageDescription,
        ]
        .compactMap { $0 }
        .joined(separator: "，")
    }
}

private struct HostPageIndicator: View {
    let count: Int
    let selectedIndex: Int

    var body: some View {
        HStack(spacing: 6) {
            ForEach(visibleRange, id: \.self) { index in
                Capsule()
                    .fill(
                        index == selectedIndex
                            ? AppThemePalette.brand
                            : Color.secondary.opacity(0.24)
                    )
                    .frame(width: index == selectedIndex ? 18 : 6, height: 6)
            }
        }
        .frame(height: 16)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("第 \(selectedIndex + 1) 台主机，共 \(count) 台")
    }

    private var visibleRange: Range<Int> {
        let visibleCount = min(count, 5)
        let lowerBound = min(
            max(selectedIndex - visibleCount / 2, 0),
            max(count - visibleCount, 0)
        )
        return lowerBound ..< lowerBound + visibleCount
    }
}

private struct HostInfoRows: View {
    let host: XboxHostSummary

    var body: some View {
        VStack(spacing: 0) {
            HostInfoRow(
                title: "主机型号",
                value: host.consoleType,
                systemImage: "desktopcomputer",
                valueColor: .primary
            )

            rowDivider

            HostInfoRow(
                title: "远程管理",
                value: capabilityTitle(host.remoteManagementEnabled),
                systemImage: "antenna.radiowaves.left.and.right",
                valueColor: capabilityColor(host.remoteManagementEnabled)
            )

            rowDivider

            HostInfoRow(
                title: "主机串流",
                value: capabilityTitle(host.consoleStreamingEnabled),
                systemImage: "play.rectangle.fill",
                valueColor: capabilityColor(host.consoleStreamingEnabled)
            )
        }
        .padding(.vertical, 3)
        .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(.white.opacity(0.16), lineWidth: 0.6)
        }
        .glassEffect(
            .regular.tint(AppThemePalette.canvasTop.opacity(0.08)),
            in: RoundedRectangle(cornerRadius: 18, style: .continuous)
        )
        .accessibilityElement(children: .contain)
    }

    private var rowDivider: some View {
        Divider()
            .padding(.leading, 58)
            .opacity(0.7)
    }

    private func capabilityTitle(_ value: Bool?) -> String {
        switch value {
        case true: "已开启"
        case false: "已关闭"
        case nil: "同步中"
        }
    }

    private func capabilityColor(_ value: Bool?) -> Color {
        switch value {
        case true: .green
        case false: .secondary
        case nil: .orange
        }
    }
}

private struct HostInfoRow: View {
    let title: String
    let value: String
    let systemImage: String
    let valueColor: Color

    var body: some View {
        HStack(spacing: 13) {
            Image(systemName: systemImage)
                .font(.system(size: 18, weight: .semibold))
                .foregroundStyle(AppThemePalette.hostControlForeground)
                .frame(width: 30)

            Text(title)
                .font(.body.weight(.medium))
                .foregroundStyle(.primary)
                .lineLimit(1)

            Spacer(minLength: 8)

            Text(value)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(valueColor)
                .lineLimit(1)
                .minimumScaleFactor(0.72)
                .multilineTextAlignment(.trailing)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 13)
        .frame(minHeight: 58)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(title)，\(value)")
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

    var tint: Color {
        switch self {
        case .powerOn: AppThemePalette.brand
        case .powerOff: AppThemePalette.hostDanger
        }
    }

    var iconColor: Color {
        switch self {
        case .powerOn: AppThemePalette.hostControlForeground
        case .powerOff: AppThemePalette.hostDanger
        }
    }
}

private struct HostActionBar: View {
    let host: XboxHostSummary
    let canStream: Bool
    let powerAction: HostPowerAction?
    let isPowerEnabled: Bool
    let onStream: () -> Void
    let onPowerOn: () -> Void
    let onPowerOff: () -> Void

    var body: some View {
        HStack(spacing: 28) {
            HostRoundActionButton(
                systemImage: "play.fill",
                tint: AppThemePalette.brand,
                iconColor: AppThemePalette.hostSuccess,
                isEnabled: canStream,
                action: onStream,
                accessibilityLabel: "开始游玩 \(host.name)",
                accessibilityHint: host.readinessDescription
            )

            if let powerAction {
                HostRoundActionButton(
                    systemImage: "power",
                    tint: powerAction.tint,
                    iconColor: powerAction.iconColor,
                    isEnabled: isPowerEnabled,
                    action: powerAction == .powerOn ? onPowerOn : onPowerOff,
                    accessibilityLabel: "\(powerAction.title) \(host.name)",
                    accessibilityHint: powerAction == .powerOn
                        ? "启动当前主机"
                        : "关闭当前主机"
                )
            }
        }
        .frame(maxWidth: 220)
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .contain)
    }
}

private struct HostRoundActionButton: View {
    let systemImage: String
    let tint: Color
    let iconColor: Color
    let isEnabled: Bool
    let action: () -> Void
    let accessibilityLabel: String
    let accessibilityHint: String

    var body: some View {
        Button(action: action) {
            Image(systemName: systemImage)
                .font(.system(size: 20, weight: .bold))
                .foregroundStyle(isEnabled ? iconColor : Color.secondary)
                .frame(width: 62, height: 62)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .glassEffect(
            .regular
                .tint(tint.opacity(isEnabled ? 0.07 : 0.025))
                .interactive(),
            in: Circle()
        )
        .overlay {
            Circle()
                .stroke(
                    tint.opacity(isEnabled ? 0.1 : 0.045),
                    lineWidth: 0.6
                )
        }
        .disabled(!isEnabled)
        .accessibilityLabel(accessibilityLabel)
        .accessibilityHint(accessibilityHint)
    }
}

private struct HostCarouselLoadingCard: View {
    var body: some View {
        VStack(spacing: 0) {
            RoundedRectangle(cornerRadius: 20, style: .continuous)
                .fill(.quaternary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .padding(.horizontal, 18)
                .padding(.vertical, 12)

            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text("客厅 Xbox")
                        .font(.title3.weight(.bold))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                        .minimumScaleFactor(0.68)
                        .layoutPriority(1)
                        .redacted(reason: .placeholder)

                    Spacer(minLength: 4)

                    HStack(spacing: 5) {
                        Circle()
                            .fill(.quaternary)
                            .frame(width: 7, height: 7)

                        Text("同步中")
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .redacted(reason: .placeholder)
                    }
                    .font(.caption.weight(.semibold))
                    .fixedSize(horizontal: true, vertical: false)
                }

                Label("可用 500 GB", systemImage: "internaldrive.fill")
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.72)
                    .redacted(reason: .placeholder)
            }
            .frame(maxWidth: .infinity, minHeight: 54, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.vertical, 5)
        }
        .compositingGroup()
        .accessibilityHidden(true)
    }
}

private struct HostListLoadingView: View {
    var body: some View {
        GeometryReader { geometry in
            ScrollView {
                VStack(spacing: 0) {
                    HostCarouselLoadingCard()
                        .frame(height: 390)
                        .padding(.horizontal, 16)
                        .padding(.top, 12)
                        .padding(.bottom, 20)

                    VStack(spacing: 0) {
                        ForEach(0 ..< 3, id: \.self) { index in
                            HStack(spacing: 13) {
                                RoundedRectangle(cornerRadius: 4, style: .continuous)
                                    .fill(.quaternary)
                                    .frame(width: 18, height: 18)
                                    .frame(width: 30)

                                Text(index == 0 ? "主机型号" : "远程管理")
                                    .font(.body.weight(.medium))
                                    .lineLimit(1)
                                    .redacted(reason: .placeholder)

                                Spacer(minLength: 12)

                                Text(index == 0 ? "Xbox Series X" : "已开启")
                                    .font(.subheadline.weight(.semibold))
                                    .lineLimit(1)
                                    .redacted(reason: .placeholder)
                            }
                            .frame(maxWidth: .infinity, minHeight: 58, alignment: .leading)
                            .padding(.horizontal, 16)

                            if index < 2 {
                                Divider()
                                    .padding(.leading, 58)
                            }
                        }
                    }
                    .padding(.vertical, 3)
                    .background(
                        Color.secondary.opacity(0.08),
                        in: RoundedRectangle(cornerRadius: 18, style: .continuous)
                    )
                    .padding(.horizontal, 16)
                    .padding(.top, 10)

                    Spacer(minLength: 24)

                    HStack(spacing: 28) {
                        ForEach(0 ..< 2, id: \.self) { _ in
                            Circle()
                                .fill(.quaternary)
                                .frame(width: 62, height: 62)
                        }
                    }
                    .frame(maxWidth: 220)
                    .frame(maxWidth: .infinity)

                    Spacer(minLength: 24)
                }
                .frame(maxWidth: .infinity, minHeight: geometry.size.height)
            }
            .scrollIndicators(.hidden)
            .safeAreaPadding(.bottom, 18)
        }
        .skeletonPulse(accessibilityLabel: "正在载入主机")
    }
}
