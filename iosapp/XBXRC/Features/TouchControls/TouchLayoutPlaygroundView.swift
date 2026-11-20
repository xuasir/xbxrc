import SwiftUI
import UniformTypeIdentifiers

struct TouchInputDocument: FileDocument {
    static var readableContentTypes: [UTType] { [.json] }
    let data: Data
    init(data: Data) { self.data = data }
    init(configuration: ReadConfiguration) throws {
        data = configuration.file.regularFileContents ?? Data()
    }
    func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper {
        FileWrapper(regularFileWithContents: data)
    }
}

struct TouchLayoutPlaygroundView: View {
    @EnvironmentObject private var settings: AppSettingsStore
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var recorder = TouchLayoutRecorder()
    @State private var opacity = 0.65
    @State private var showEvents = false
    @State private var showCapabilities = false
    @State private var showSampling = false
    @State private var showExporter = false
    @State private var document: TouchInputDocument?
    @State private var exportError: String?

    var body: some View {
        VStack(spacing: 0) {
            toolbar
            ZStack(alignment: .top) {
                TouchPlaygroundBackdrop()
                TouchLayoutSurface(
                    input: recorder.input, opacity: opacity,
                    onPress: { MobileInteractionFeedback.play(settings.mobile) })
                if showEvents {
                    Text(recorder.recentEvents.reversed().joined(separator: "\n"))
                        .font(.system(size: 10, design: .monospaced))
                        .frame(width: 260, height: 96, alignment: .topLeading)
                        .padding(10).background(
                            .regularMaterial, in: RoundedRectangle(cornerRadius: 8)
                        )
                        .padding(.top, 12).allowsHitTesting(false)
                        .accessibilityIdentifier("touch-input-event-list")
                }
                if let error = recorder.loadError {
                    Text(error).padding().foregroundStyle(.red)
                }
            }
            .clipped()
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            inputPanel
        }
        .appThemeCanvas()
        .onDisappear { recorder.input.cancelAll(reason: "pageClosed") }
        .onChange(of: scenePhase) { _, phase in
            if phase != .active { recorder.input.cancelAll(reason: "background") }
        }
        .sheet(isPresented: $showCapabilities) { capabilityPanel }
        .sheet(isPresented: $showSampling) { samplingPanel }
        .fileExporter(
            isPresented: $showExporter, document: document, contentType: .json,
            defaultFilename: "touch-input"
        ) { result in
            if case .failure(let error) = result { exportError = error.localizedDescription }
        }
        .alert(
            "导出记录",
            isPresented: Binding(get: { exportError != nil }, set: { _ in exportError = nil })
        ) {
            Button("确定") { exportError = nil }
        } message: {
            Text(exportError ?? "")
        }
    }

    private var toolbar: some View {
        HStack(spacing: 12) {
            Button {
                dismiss()
            } label: {
                Image(systemName: "xmark").frame(width: 44, height: 44)
            }
            .accessibilityLabel("关闭测试").accessibilityIdentifier("touch-playground-close")
            Menu {
                ForEach(recorder.choices) { choice in
                    Button(choice.title) { recorder.select(choice.id) }
                        .accessibilityIdentifier("touch-layout-choice-\(choice.id)")
                }
            } label: {
                HStack(spacing: 5) {
                    Text(recorder.selected?.title ?? "布局与输入测试").lineLimit(1)
                    Image(systemName: "chevron.down").font(.caption)
                }
                .frame(minHeight: 44)
            }
            .accessibilityIdentifier("touch-layout-picker")
            Spacer(minLength: 0)
            Menu {
                Button("能力报告", systemImage: "list.bullet.rectangle") { showCapabilities = true }
                Button("采样诊断", systemImage: "speedometer") {
                    recorder.refreshPresentation()
                    showSampling = true
                }
                Button(showEvents ? "收起记录" : "显示记录", systemImage: "waveform.path") {
                    showEvents.toggle()
                }
                Button("清空输入", systemImage: "arrow.counterclockwise") { recorder.clear() }
                Button("导出 JSON", systemImage: "square.and.arrow.up") {
                    recorder.input.cancelAll(reason: "export")
                    do {
                        document = TouchInputDocument(data: try recorder.export())
                        showExporter = true
                    } catch { exportError = error.localizedDescription }
                }
            } label: {
                Image(systemName: "ellipsis").frame(width: 44, height: 44)
            }
            .accessibilityLabel("测试工具").accessibilityIdentifier("touch-playground-tools")
        }
        .padding(.horizontal, 8)
        .appFont(.body, weight: .semibold)
    }

    private var inputPanel: some View {
        let state = recorder.latest?.gamepad ?? .neutral
        let buttons = TouchGamepadMapping.masks.keys.sorted().filter {
            state.buttonMask & (TouchGamepadMapping.masks[$0] ?? 0) != 0
        }.map { TouchGamepadMapping.title($0) }.joined(separator: " · ")
        return VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 10) {
                Text(buttons.isEmpty ? "按键 —" : buttons)
                Spacer(minLength: 0)
                Text("触点 \(recorder.latest?.pointerCount ?? 0)")
                Text("\(recorder.latest?.activeLayerIDs.joined(separator: ", ") ?? "")")
            }
            .accessibilityElement(children: .combine).accessibilityIdentifier("touch-input-buttons")
            Text(
                String(
                    format: "L %+.2f %+.2f    R %+.2f %+.2f    LT %.2f  RT %.2f",
                    state.leftThumbX, state.leftThumbY, state.rightThumbX, state.rightThumbY,
                    state.leftTrigger, state.rightTrigger)
            )
            .font(.system(size: 11, design: .monospaced))
            .accessibilityIdentifier("touch-input-axes")
            HStack {
                Text("透明度 \(Int(opacity * 100))%").font(.caption)
                Slider(value: $opacity, in: 0.2...1, step: 0.05)
                    .accessibilityLabel("触控透明度")
                Text("\(recorder.retainedFrameCount) 批").font(.caption).monospacedDigit()
            }
            if let sampling = recorder.sampling {
                Text(
                    String(
                        format: "源 %.0f Hz · 回调 %.0f Hz · 处理 P95 %.2f ms",
                        sampling.movementIntervals.frequencyHz,
                        sampling.callbackIntervals.frequencyHz, sampling.processing.p95Milliseconds)
                )
                .font(.system(size: 10, design: .monospaced)).lineLimit(1)
                .accessibilityIdentifier("touch-input-sampling")
            }
            Text(recorder.recentEvents.last ?? "触摸控件查看输入")
                .font(.system(size: 10, design: .monospaced)).lineLimit(1)
                .accessibilityIdentifier("touch-input-events")
        }
        .appFont(.caption)
        .foregroundStyle(AppThemePalette.textSecondary)
        .padding(.horizontal, 16).padding(.vertical, 8)
    }

    private var capabilityPanel: some View {
        NavigationStack {
            List {
                if let choice = recorder.selected {
                    Section("布局") {
                        LabeledContent("名称", value: choice.layout.name)
                        LabeledContent("作者", value: choice.layout.author ?? "XBXRC")
                        LabeledContent("格式", value: choice.layout.report.format)
                        Text(choice.layout.report.schema).font(.caption)
                        Text(choice.layout.report.controlTypes.joined(separator: " · "))
                        Text(choice.layout.report.behaviors.joined(separator: " · ")).font(.caption)
                    }
                    Section("能力检查") {
                        if choice.layout.report.issues.isEmpty { Text("此布局能力已覆盖") }
                        ForEach(choice.layout.report.issues) { issue in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(issue.message)
                                Text(issue.path).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
            .navigationTitle("能力报告")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("完成") { showCapabilities = false }
                }
            }
        }
    }

    private var samplingPanel: some View {
        NavigationStack {
            List {
                if let stats = recorder.sampling {
                    Section("采样与更新") {
                        LabeledContent("源移动采样", value: hz(stats.movementIntervals))
                        LabeledContent("输入回调", value: hz(stats.callbackIntervals))
                        LabeledContent("手柄状态变化", value: hz(stats.stateChangeIntervals))
                        LabeledContent("控件绘制", value: hz(stats.renderIntervals))
                        LabeledContent(
                            "源样本 / 合并历史",
                            value:
                                "\(stats.acceptedSamples) / \(stats.coalescedSamples)")
                        Text("保持连续移动以观察采样率；绘制随输入与反馈发生。诊断面板最高每秒刷新十次。")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    Section("间隔与延迟 · ms") {
                        timingRow("源移动间隔", stats.movementIntervals)
                        timingRow("源 → 接收", stats.sourceToReceipt)
                        timingRow("源 → 手柄状态", stats.sourceToState)
                        timingRow("单批处理", stats.processing)
                        Text("P95 / P99 / 最大值，取最近至多 4096 项。")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    Section("边沿与记录") {
                        LabeledContent(
                            "按下 / 释放 / 取消",
                            value:
                                "\(stats.beganCount) / \(stats.endedCount) / \(stats.cancelledCount)"
                        )
                        LabeledContent(
                            "重复 / 乱序 / 异常",
                            value:
                                "\(stats.duplicateSamples) / \(stats.outOfOrderSamples) / \(stats.invalidSamples)"
                        )
                        LabeledContent(
                            "无活动指针 / 超限",
                            value:
                                "\(stats.orphanSamples) / \(stats.pointerLimitSamples)")
                        LabeledContent(
                            "保留批次 / 容量淘汰",
                            value:
                                "\(recorder.retainedFrameCount) / \(recorder.evictedFrameCount)")
                    }
                    Section("最近操作指针") {
                        ForEach(stats.pointers, id: \.pointerID) { pointer in
                            LabeledContent(
                                "#\(pointer.pointerID) · \(pointer.sampleCount) 点",
                                value: hz(pointer.movementIntervals))
                        }
                    }
                }
            }
            .accessibilityIdentifier("touch-sampling-report")
            .navigationTitle("采样诊断")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("完成") { showSampling = false }
                }
            }
        }
    }

    private func hz(_ timing: TouchTimingSummary) -> String {
        String(format: "%.1f Hz", timing.frequencyHz)
    }

    private func timingRow(_ title: String, _ timing: TouchTimingSummary) -> some View {
        LabeledContent(
            title,
            value: String(
                format: "%.2f / %.2f / %.2f",
                timing.p95Milliseconds, timing.p99Milliseconds, timing.maximumMilliseconds))
    }
}

/// 本地画面让透明控件的遮挡程度在离线测试中可见。
private struct TouchPlaygroundBackdrop: View {
    var body: some View {
        GeometryReader { geometry in
            ZStack {
                LinearGradient(
                    colors: [
                        Color(red: 0.17, green: 0.24, blue: 0.29),
                        Color(red: 0.06, green: 0.09, blue: 0.10),
                    ],
                    startPoint: .top, endPoint: .bottom)
                Path { path in
                    let size = geometry.size
                    path.move(to: CGPoint(x: size.width * 0.47, y: size.height * 0.35))
                    path.addLine(to: CGPoint(x: size.width * 0.53, y: size.height * 0.35))
                    path.addLine(to: CGPoint(x: size.width * 0.78, y: size.height))
                    path.addLine(to: CGPoint(x: size.width * 0.22, y: size.height))
                    path.closeSubpath()
                }.fill(.white.opacity(0.055))
                VStack(spacing: 6) {
                    Image(systemName: "gamecontroller").font(.system(size: 26, weight: .ultraLight))
                    Text("布局与输入测试").font(.system(size: 12))
                    Text("本地采集").font(.system(size: 10))
                }.foregroundStyle(.white.opacity(0.35))
            }
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}
