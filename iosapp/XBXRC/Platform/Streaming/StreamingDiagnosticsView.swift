import SwiftUI
import UIKit

struct StreamingLiveMetricsBar: View {
    let store: StreamingFeatureStore
    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { _ in
            StreamingRuntimeMetricsBar(
                model: StreamingDiagnosticsModel(
                    diagnostics: store.diagnostics, state: store.state, picture: store.picture))
        }
    }
}

struct StreamingRuntimeMetricsBar: View {
    let model: StreamingDiagnosticsModel

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 12) {
                renderMode
                resolution
                fps
                bitrate
                latency
                loss
            }.fixedSize()
            VStack(spacing: 5) {
                HStack(spacing: 12) {
                    renderMode
                    resolution
                    fps
                }
                HStack(spacing: 12) {
                    bitrate
                    latency
                    loss
                }
            }.fixedSize()
            VStack(spacing: 5) {
                HStack(spacing: 12) {
                    renderMode
                    resolution
                }
                HStack(spacing: 12) {
                    fps
                    bitrate
                }
                HStack(spacing: 12) {
                    latency
                    loss
                }
            }.fixedSize()
        }
        .font(.system(size: 12, weight: .medium))
        .monospacedDigit()
        .foregroundStyle(.white.opacity(0.9))
        .padding(.horizontal, 10).padding(.vertical, 6)
        .background(.black.opacity(0.62), in: RoundedRectangle(cornerRadius: 6))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("compact-performance-overlay")
        .allowsHitTesting(false)
    }

    private var renderMode: some View { Text(LocalizedStringKey(model.renderLabel)) }
    private var resolution: some View {
        Text(verbatim: model.resolution).accessibilityIdentifier("stream-metric-resolution")
    }
    private var fps: some View {
        Text(verbatim: "\(model.presentFps) FPS").accessibilityIdentifier(
            "stream-metric-present-fps")
    }
    @ViewBuilder private var bitrate: some View {
        if let notice = model.notice {
            Text(LocalizedStringKey(notice)).foregroundStyle(.orange)
        } else {
            Text(verbatim: model.videoBitrate).accessibilityIdentifier(
                "stream-metric-video-bitrate")
        }
    }
    private var latency: some View {
        Text(verbatim: "RTT \(model.rtt)").accessibilityIdentifier("stream-metric-rtt")
    }
    private var loss: some View {
        HStack(spacing: 3) {
            Text("丢包")
            Text(verbatim: model.packetLoss)
        }.accessibilityElement(children: .combine).accessibilityIdentifier(
            "stream-metric-packet-loss")
    }
}

struct StreamingDiagnosticsView: View {
    let store: StreamingFeatureStore
    let onClose: () -> Void
    @State private var group: StreamingDiagnosticGroup = .network
    @State private var copied = false

    var body: some View {
        GeometryReader { geometry in
            TimelineView(.periodic(from: .now, by: 1)) { _ in
                let model = StreamingDiagnosticsModel(
                    diagnostics: store.diagnostics, state: store.state, picture: store.picture)
                StreamingDiagnosticsPanel(
                    model: model, group: $group, copied: $copied,
                    compact: geometry.size.height < 420)
            }
        }
        .onAppear {
            group =
                StreamingDiagnosticsModel(
                    diagnostics: store.diagnostics, state: store.state, picture: store.picture
                ).initialGroup
        }
        .onChange(of: group) { _, _ in copied = false }
    }
}

struct StreamingDiagnosticsPanel: View {
    let model: StreamingDiagnosticsModel
    @Binding var group: StreamingDiagnosticGroup
    @Binding var copied: Bool
    var compact = false

    var body: some View {
        VStack(spacing: compact ? 6 : 12) {
            Picker("诊断分组", selection: $group) {
                ForEach(StreamingDiagnosticGroup.allCases) { tab in
                    Text(LocalizedStringKey(tab.title)).tag(tab)
                }
            }
            .pickerStyle(.segmented)
            .accessibilityIdentifier("stream-diagnostic-groups")
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(model.rows(group)) { row in
                        HStack(alignment: .firstTextBaseline, spacing: 16) {
                            Text(LocalizedStringKey(row.title))
                                .foregroundStyle(AppThemePalette.textSecondary)
                            Spacer(minLength: 0)
                            Text(LocalizedStringKey(row.value))
                                .monospacedDigit().multilineTextAlignment(.trailing)
                                .textSelection(.enabled)
                        }
                        .appFont(.caption)
                        .padding(.vertical, compact ? 6 : 12).padding(.horizontal, 4)
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("stream-diagnostic-\(row.id)")
                        Divider().opacity(0.35)
                    }
                }
                .padding(.horizontal, 12)
            }
            .appSurface(radius: 10)
            .accessibilityIdentifier("stream-diagnostic-rows")
            Text(LocalizedStringKey(group.hint))
                .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
                .frame(maxWidth: .infinity, alignment: .leading)
            HStack {
                Button {
                    UIPasteboard.general.string = model.clipboardSummary
                    copied = true
                } label: {
                    Label("复制诊断摘要", systemImage: "doc.on.doc")
                }
                .accessibilityIdentifier("stream-diagnostic-copy")
                if copied { Text("已复制").appFont(.caption) }
                Spacer()
            }
            .frame(minHeight: compact ? 32 : 44)
        }
        .padding(compact ? 10 : 16)
    }
}

struct StreamingDiagnosticsFloatingPanel: View {
    let store: StreamingFeatureStore
    let onClose: () -> Void

    var body: some View {
        GeometryReader { geometry in
            let size = StreamingDiagnosticsPlacement.size(in: geometry.size)
            VStack(spacing: 0) {
                HStack {
                    Text("诊断指标").font(.subheadline.weight(.semibold))
                    Spacer()
                    StreamingDiagnosticsCloseButton(action: onClose)
                }.padding(.leading, 16).padding(.trailing, 4)
                StreamingDiagnosticsView(store: store, onClose: onClose)
            }
            .foregroundStyle(.white)
            .environment(\.colorScheme, .dark)
            .frame(width: size.width, height: size.height)
            .background(.black.opacity(0.72), in: RoundedRectangle(cornerRadius: 12))
            .clipShape(RoundedRectangle(cornerRadius: 12))
            .shadow(color: .black.opacity(0.2), radius: 4, y: 2)
            .accessibilityElement(children: .contain)
            .position(x: geometry.size.width - 14 - size.width / 2, y: 14 + size.height / 2)
            .accessibilityIdentifier("stream-diagnostics-floating-panel")
        }
    }
}

private struct StreamingDiagnosticsCloseButton: View {
    let action: () -> Void

    var body: some View {
        Button("关闭诊断指标", systemImage: "xmark", action: action)
            .labelStyle(.iconOnly)
            .frame(minWidth: 44, minHeight: 44)
            .accessibilityIdentifier("stream-diagnostics-close")
    }
}

enum StreamingDiagnosticsPlacement {
    static func size(in available: CGSize) -> CGSize {
        let portrait = available.height > available.width
        return CGSize(
            width: max(1, min(420, available.width - 28)),
            height: max(
                1,
                min(
                    420, available.height - 28, max(280, available.height * (portrait ? 0.46 : 0.8))
                )))
    }
}

#if DEBUG
    /// 无远端会话的界面验收入口，直接装配正式触屏、控制菜单与诊断浮层。
    struct StreamingPanelsUITestView: View {
        @EnvironmentObject private var settings: AppSettingsStore
        @EnvironmentObject private var store: StreamingFeatureStore
        @State private var revealToken = 0

        var body: some View {
            ZStack(alignment: .top) {
                LinearGradient(
                    colors: [.blue.opacity(0.65), .green.opacity(0.45), .black],
                    startPoint: .topLeading, endPoint: .bottomTrailing
                ).ignoresSafeArea()
                TouchGamepadOverlay(input: store.input)
                if settings.mobile.compactPerformanceOverlay {
                    StreamingRuntimeMetricsBar(
                        model: StreamingDiagnosticsModel(
                            diagnostics: store.diagnostics, state: store.state,
                            picture: store.picture)
                    )
                    .padding(.horizontal, 64)
                }
                StreamingControlsOverlay(
                    store: store, revealToken: revealToken, revealSource: .accessibility,
                    autoHideAfter: nil)
            }
            .onAppear { store.prepareControlsUITestPreview() }
            .background {
                StreamingControlsGestureLayer { revealToken += 1 }
            }
            .overlay(alignment: .top) {
                // DEBUG 夹具提供无障碍唤出入口，正式串流使用双指手势。
                Button("测试唤出串流菜单") { revealToken += 1 }
                    .accessibilityIdentifier("stream-test-reveal")
            }
        }
    }
#endif
