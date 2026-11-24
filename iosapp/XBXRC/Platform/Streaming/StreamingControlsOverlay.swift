import Combine
import SwiftUI
import UIKit

struct StreamingControlsOverlay: View {
    // 快捷栏只订阅操作状态，统计采样交给指标叶子视图。
    let store: StreamingFeatureStore
    let revealToken: Int
    let revealSource: StreamingControlsRevealSource
    var autoHideAfter: Duration? = .seconds(5)
    @EnvironmentObject private var settings: AppSettingsStore
    @State private var controls = StreamingControlsState()
    @State private var audioMuted = false
    @State private var microphoneEnabled = false
    @State private var microphonePending = false
    @State private var videoContentMode: StreamingVideoContentMode = .fit
    @State private var capabilities = StreamingControlCapabilities.unavailable
    @State private var message: String?
    @State private var confirmExit = false
    @State private var confirmPowerOff = false
    @State private var capturing = false
    @State private var exporting = false
    @State private var switchingLayout = false
    @State private var sharedTrace: StreamingTraceShareItem?
    @State private var exportError: String?
    @State private var autoHideTask: Task<Void, Never>?
    @State private var messageTask: Task<Void, Never>?

    var body: some View {
        ZStack {
            if controls.quickControlsVisible {
                VStack {
                    HStack(spacing: 8) {
                        dockButton(
                            systemName: "line.3.horizontal", label: "打开串流菜单",
                            identifier: "stream-controls-open"
                        ) {
                            reveal(source: .button)
                        }
                        .streamingQuickControlSurface()
                        StreamingTouchQuickSwitch(
                            controller: store.touchLayout, session: store.input)
                        Spacer(minLength: 0)
                    }
                    Spacer()
                }
                .padding(16)
            }
            if let message {
                VStack {
                    Text(message)
                        .font(.subheadline.weight(.semibold)).foregroundStyle(.white)
                        .lineLimit(2).padding(.horizontal, 14).padding(.vertical, 9)
                        .background(.black.opacity(0.5), in: Capsule())
                    Spacer()
                }.padding(.top, 18).padding(.horizontal, 64)
            }
            if controls.dockVisible {
                GeometryReader { geometry in
                    HStack(alignment: .center) {
                        rail(
                            height: min(
                                geometry.size.height * 0.5, capabilities.canPowerOff ? 368 : 324)
                        ) {
                            operationButtons
                        }
                        .accessibilityIdentifier("stream-operations-rail")
                        Spacer(minLength: 0)
                        rail(height: min(geometry.size.height * 0.5, 236)) {
                            diagnosticButtons
                        }
                        .accessibilityIdentifier("stream-diagnostics-rail")
                    }
                    .padding(.horizontal, 12)
                    .frame(maxHeight: .infinity)
                }
                .transition(.opacity)
            }
            if controls.surface == .debugSheet {
                StreamingDiagnosticsFloatingPanel(store: store, onClose: resumeDock)
            }
            if controls.surface == .touchLayoutPicker {
                StreamingTouchLayoutPickerPanel(controller: store.touchLayout, onClose: resumeDock)
            }
            if controls.surface == .pictureSheet {
                StreamingPictureControlsPanel(store: store, onClose: resumeDock)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityAction(named: "打开串流控制") { reveal(source: .accessibility) }
        .onChange(of: revealToken) { _, _ in
            guard controls.surface == .hidden || controls.dockVisible else { return }
            if revealSource == .blankTap {
                revealQuickControls()
                return
            }
            controls.dockVisible ? hideDock() : reveal(source: revealSource)
        }
        .onReceive(store.touchLayout.$pendingSelection.map { $0 != nil }.removeDuplicates()) {
            pending in
            switchingLayout = pending
            scheduleAutoHide()
        }
        .onReceive(store.$audioMuted.removeDuplicates()) { audioMuted = $0 }
        .onReceive(store.$microphoneEnabled.removeDuplicates()) { microphoneEnabled = $0 }
        .onReceive(store.$microphonePending.removeDuplicates()) { microphonePending = $0 }
        .onReceive(store.$videoContentMode.removeDuplicates()) { videoContentMode = $0 }
        .onReceive(
            store.$state.combineLatest(store.$diagnostics, store.$streamTarget)
                .map { state, diagnostics, target in
                    StreamingControlCapabilities(
                        canMuteAudio: state == .playing && diagnostics.audioTrackReady,
                        canSendText: state == .playing && target == .home,
                        canPowerOff: state == .playing && target == .home)
                }.removeDuplicates()
        ) { capabilities = $0 }
        .onReceive(store.$controlMessage.removeDuplicates()) { value in
            message = value
            messageTask?.cancel()
            guard let value else { return }
            scheduleAutoHide()
            messageTask = Task { @MainActor in
                try? await Task.sleep(for: .seconds(2))
                guard !Task.isCancelled else { return }
                store.clearControlMessage(ifMatching: value)
            }
        }
        .onReceive(store.input.$diagnosticsRevealToken.dropFirst()) { _ in
            guard controls.surface == .hidden || controls.dockVisible else { return }
            showDiagnostics()
        }
        .alert("结束串流？", isPresented: $confirmExit) {
            Button("取消", role: .cancel) { resumeDock() }
            Button("结束串流", role: .destructive) { store.stop() }
        }
        .alert("关闭主机并结束串流？", isPresented: $confirmPowerOff) {
            Button("取消", role: .cancel) { resumeDock() }
            Button("关闭主机", role: .destructive) { store.powerOffAndStop() }
        }
        .sheet(item: $sharedTrace, onDismiss: resumeDock) { item in
            StreamingActivityView(url: item.url)
        }
        .alert(
            "Trace 导出失败",
            isPresented: Binding(get: { exportError != nil }, set: { if !$0 { exportError = nil } })
        ) {
            Button("知道了", role: .cancel) {
                exportError = nil
                resumeDock()
            }
        } message: {
            Text(exportError ?? "当前没有可导出的 Trace")
        }
        .onChange(of: controls.surface) { _, value in
            store.input.setSuppressed(value != .hidden)
        }
        .task(id: controls.dockVisible) {
            guard controls.dockVisible else { return }
            while !Task.isCancelled {
                capturing = IOSRuntimeTrace.isCapturing
                try? await Task.sleep(for: .seconds(1))
            }
        }
        .onAppear { scheduleAutoHide() }
        .onDisappear {
            cancelAutoHide()
            messageTask?.cancel()
            store.input.setSuppressed(false)
        }
    }

    private var operationButtons: some View {
        Group {
            dockButton(
                systemName: "gamecontroller", label: "切换触屏布局",
                identifier: "stream-touch-layout-open"
            ) {
                cancelAutoHide()
                controls.present(.touchLayoutPicker)
            }
            dockButton(
                systemName: "slider.horizontal.3", label: "画面设置",
                identifier: "stream-picture-settings"
            ) {
                cancelAutoHide()
                controls.present(.pictureSheet)
            }
            dockButton(
                systemName: audioMuted ? "speaker.slash.fill" : "speaker.wave.2.fill",
                label: audioMuted ? "恢复声音" : "静音", identifier: "stream-audio-toggle",
                active: audioMuted
            ) {
                store.setAudioMuted(!audioMuted)
                scheduleAutoHide()
            }.disabled(!capabilities.canMuteAudio)
            dockButton(
                systemName: microphoneEnabled ? "mic.fill" : "mic.slash",
                label: microphonePending ? "取消开启麦克风" : (microphoneEnabled ? "关闭麦克风" : "开启麦克风"),
                identifier: "stream-microphone-toggle",
                active: microphoneEnabled || microphonePending
            ) {
                store.setMicrophoneEnabled(!microphoneEnabled && !microphonePending)
                scheduleAutoHide()
            }
            dockButton(
                systemName: videoContentMode == .fit
                    ? "arrow.down.right.and.arrow.up.left" : "arrow.up.left.and.arrow.down.right",
                label: "画面\(videoContentMode.title)", identifier: "stream-aspect-toggle"
            ) {
                store.toggleVideoContentMode()
                scheduleAutoHide()
            }
            if capabilities.canSendText {
                PasteButton(payloadType: String.self) { values in
                    if let text = values.first { store.sendText(String(text.prefix(512))) }
                    scheduleAutoHide()
                }
                .labelStyle(.iconOnly).buttonBorderShape(.circle).tint(.white.opacity(0.18))
                .frame(width: 44, height: 44)
                .accessibilityLabel("粘贴并发送文字")
                .accessibilityIdentifier("stream-paste-text")
            }
            dockButton(
                systemName: "rectangle.portrait.and.arrow.right", label: "结束串流",
                identifier: "stream-exit"
            ) {
                cancelAutoHide()
                controls.present(.confirmingExit)
                confirmExit = true
            }
            if capabilities.canPowerOff {
                dockButton(systemName: "power", label: "关闭主机并结束", identifier: "stream-power-off") {
                    cancelAutoHide()
                    controls.present(.confirmingPowerOff)
                    confirmPowerOff = true
                }
            }
        }
    }

    private var diagnosticButtons: some View {
        Group {
            dockButton(
                systemName: "chart.bar.xaxis",
                label: settings.mobile.compactPerformanceOverlay ? "隐藏运行指标" : "显示运行指标",
                identifier: "stream-metrics-toggle",
                active: settings.mobile.compactPerformanceOverlay
            ) {
                settings.mobile.compactPerformanceOverlay.toggle()
                scheduleAutoHide()
            }
            dockButton(
                systemName: "waveform.path.ecg", label: "显示诊断指标",
                identifier: "stream-diagnostics-menu",
                action: showDiagnostics)
            dockButton(
                systemName: capturing ? "stop.circle" : "record.circle",
                label: capturing ? "停止并导出诊断录制" : "开始诊断录制", identifier: "stream-record-toggle",
                active: capturing
            ) {
                performExport {
                    if IOSRuntimeTrace.isCapturing { return await IOSRuntimeTrace.stopCapture() }
                    try await IOSRuntimeTrace.startCapture()
                    return nil
                }
            }.disabled(exporting)
            dockButton(
                systemName: "square.and.arrow.up", label: "导出运行日志",
                identifier: "stream-trace-export"
            ) {
                performExport { try await IOSRuntimeTrace.prepareExport(allFiles: false) }
            }.disabled(exporting)
            dockButton(
                systemName: "xmark", label: "收起串流菜单", identifier: "stream-controls-close",
                action: hideDock)
        }
    }

    private func rail<Content: View>(height: CGFloat, @ViewBuilder content: () -> Content)
        -> some View
    {
        ScrollView(.vertical) {
            VStack(spacing: 0, content: content).padding(4)
        }
        .scrollIndicators(.hidden)
        .frame(width: 52, height: max(44, height))
        // 菜单与诊断共用公共突出浮层的背景和降低透明度回退。
        .appSurface(radius: 14, prominent: true, forceDark: true)
    }

    private func dockButton(
        systemName: String, label: String, identifier: String, active: Bool = false,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: systemName)
                .font(.system(size: 19, weight: .medium)).foregroundStyle(.white)
                .shadow(color: .black.opacity(0.6), radius: 1)
                .frame(width: 44, height: 44)
                .background(
                    active ? AppThemePalette.brand.opacity(0.3) : .clear,
                    in: RoundedRectangle(cornerRadius: 10)
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain).accessibilityLabel(label).accessibilityIdentifier(identifier)
        .accessibilityValue(active ? "已开启" : "")
    }

    private func reveal(source: StreamingControlsRevealSource) {
        controls.reveal()
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        scheduleAutoHide()
        IOSRuntimeTrace.state(
            domain: "ios-streaming", event: "streamControlsVisibilityChanged",
            payload: ["visible": .bool(true), "source": .string(source.rawValue)],
            dimension: .input,
            importance: .debug)
    }

    private func revealQuickControls() {
        guard controls.surface == .hidden else { return }
        controls.revealQuickControls()
        scheduleAutoHide()
        IOSRuntimeTrace.state(
            domain: "ios-streaming", event: "streamQuickControlsVisibilityChanged",
            payload: ["visible": .bool(true), "source": .string("blankTap")],
            dimension: .input,
            importance: .debug)
    }

    private func resumeDock() {
        controls.reveal()
        scheduleAutoHide()
    }
    private func showDiagnostics() {
        cancelAutoHide()
        controls.present(.debugSheet)
    }
    private func hideDock() {
        cancelAutoHide()
        controls.hide()
        scheduleAutoHide()
    }

    private func performExport(_ operation: @escaping @MainActor () async throws -> URL?) {
        guard !exporting else { return }
        cancelAutoHide()
        exporting = true
        Task { @MainActor in
            defer {
                exporting = false
                capturing = IOSRuntimeTrace.isCapturing
                scheduleAutoHide()
            }
            do {
                if let url = try await operation() {
                    controls.present(.sharing)
                    sharedTrace = StreamingTraceShareItem(url: url)
                }
            } catch {
                controls.present(.sharing)
                exportError = CloudLibraryDiagnostics.safeError(error)
            }
        }
    }

    private func scheduleAutoHide() {
        cancelAutoHide()
        guard controls.dockVisible || controls.quickControlsVisible,
            !exporting, !switchingLayout, let autoHideAfter
        else { return }
        autoHideTask = Task { @MainActor in
            try? await Task.sleep(for: autoHideAfter)
            guard !Task.isCancelled, !controls.suspendsAutoHide, !switchingLayout else { return }
            controls.hideAutomatically()
            IOSRuntimeTrace.state(
                domain: "ios-streaming", event: "streamControlsVisibilityChanged",
                payload: ["visible": .bool(false), "source": .string("autoHide")],
                dimension: .input,
                importance: .debug)
        }
    }

    private func cancelAutoHide() {
        autoHideTask?.cancel()
        autoHideTask = nil
    }
}

private struct StreamingPictureControlsPanel: View {
    @EnvironmentObject private var settings: AppSettingsStore
    let store: StreamingFeatureStore
    let onClose: () -> Void
    @State private var picture: MobilePicturePreferences
    @State private var contentMode: StreamingVideoContentMode

    init(store: StreamingFeatureStore, onClose: @escaping () -> Void) {
        self.store = store
        self.onClose = onClose
        _picture = State(initialValue: store.picture)
        _contentMode = State(initialValue: store.videoContentMode)
    }

    var body: some View {
        GeometryReader { geometry in
            let width = min(600, max(1, geometry.size.width - 28))
            let height = max(1, min(geometry.size.height * 0.5, geometry.size.height - 28))
            NavigationStack {
                MobilePictureSettingsView(
                    picture: Binding(
                        get: { picture },
                        set: {
                            picture = $0
                            store.picture = $0
                        }),
                    videoContentMode: Binding(
                        get: { contentMode },
                        set: {
                            contentMode = $0
                            store.setVideoContentMode($0)
                        }),
                    onSaveDefault: { settings.mobile.picture = store.picture },
                    onRestoreDefault: { store.picture = settings.mobile.picture },
                    showsBack: false
                )
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("完成", systemImage: "checkmark", action: onClose)
                            .foregroundStyle(AppThemePalette.textPrimary).labelStyle(.iconOnly)
                            .frame(minWidth: 44, minHeight: 44)
                            .accessibilityIdentifier("stream-picture-close")
                    }.sharedBackgroundVisibility(.hidden)
                }
            }
            .frame(width: width, height: height)
            .appSurface(radius: 12, prominent: true, forceDark: true)
            .clipShape(RoundedRectangle(cornerRadius: 12))
            .position(x: geometry.size.width / 2, y: geometry.size.height - height / 2)
            .accessibilityIdentifier("stream-picture-panel")
        }
        .ignoresSafeArea(.container, edges: .bottom)
        .onReceive(store.$picture.removeDuplicates()) { picture = $0 }
        .onReceive(store.$videoContentMode.removeDuplicates()) { contentMode = $0 }
    }
}

struct StreamingActivityView: UIViewControllerRepresentable {
    let url: URL
    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: [url], applicationActivities: nil)
    }
    func updateUIViewController(_ uiViewController: UIActivityViewController, context: Context) {}
}

private struct StreamingTraceShareItem: Identifiable {
    let id = UUID()
    let url: URL
}
