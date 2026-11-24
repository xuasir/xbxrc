import SwiftUI
import UIKit

struct TouchLayoutEditorView: View {
    @EnvironmentObject private var settings: AppSettingsStore
    @Environment(\.dismiss) private var dismiss
    @State private var draft: TouchLayoutTemplate
    @State private var orientation: TouchLayoutOrientation = .portrait
    @State private var showDelete = false
    @State private var showLimit = false
    private let originalID: String?

    init(template: TouchLayoutTemplate?, appearance: TouchLayoutAppearance) {
        originalID = template?.id
        _draft = State(initialValue: template ?? TouchLayoutTemplate(appearance: appearance))
    }

    var body: some View {
        TouchLayoutWorkspace(
            title: "自定义布局", saveIdentifier: "touch-template-save", onSave: { save() },
            orientation: $orientation, appearance: draft.appearance,
            configuration: .init(
                identity: draft.id, positions: draft.positions(for: orientation),
                scale: draft.appearance.scale),
            onMove: { id, point, size, insets in
                draft.setPosition(
                    TouchLayoutGeometry.normalizedPosition(point, size: size, insets: insets),
                    for: id, orientation: orientation)
            }
        ) {
            LabeledContent("模板名称") {
                TextField("输入名称", text: $draft.name)
                    .multilineTextAlignment(.trailing)
                    .submitLabel(.done)
                    .accessibilityIdentifier("touch-template-name")
            }
            Text("拖动按键调整位置，横屏和竖屏分别保存。")
                .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
            AppSettingsActionButton("恢复位置") { draft.resetPositions(for: orientation) }
                .accessibilityIdentifier("touch-template-reset")
            if originalID != nil {
                AppSettingsActionButton("另存为新模板") { save(asNew: true) }
                    .accessibilityIdentifier("touch-template-save-new")
                AppSettingsActionButton("删除模板", role: .destructive) { showDelete = true }
                    .accessibilityIdentifier("touch-template-delete")
            }
        }
        .confirmationDialog("删除此本地模板？", isPresented: $showDelete, titleVisibility: .visible) {
            Button("删除模板", role: .destructive) {
                if let originalID { settings.mobile.deleteTouchTemplate(originalID) }
                dismiss()
            }
            .accessibilityIdentifier("touch-template-confirm-delete")
        }
        .alert("本地模板已达上限", isPresented: $showLimit) {
            Button("完成", role: .cancel) {}
        } message: {
            Text("最多保存 20 个模板，可更新或删除已有模板。")
        }
    }

    private func save(asNew: Bool = false) {
        var preferences = settings.mobile
        if preferences.saveTouchTemplate(draft, asNew: asNew) {
            settings.mobile = preferences
            dismiss()
        } else {
            showLimit = true
        }
    }
}

struct TouchLayoutAdjustmentView: View {
    @EnvironmentObject private var settings: AppSettingsStore
    @Environment(\.dismiss) private var dismiss
    @State private var appearance: TouchLayoutAppearance
    @State private var orientation: TouchLayoutOrientation = .portrait
    private let template: TouchLayoutTemplate?
    private let layoutID: String

    init(template: TouchLayoutTemplate?, appearance: TouchLayoutAppearance) {
        self.template = template
        layoutID = template?.id ?? TouchLayoutTemplate.standardID
        _appearance = State(initialValue: appearance)
    }

    var body: some View {
        TouchLayoutWorkspace(
            title: "触屏手柄调整", saveIdentifier: "touch-appearance-save",
            onSave: save,
            orientation: $orientation, appearance: appearance,
            configuration: .init(
                identity: layoutID, positions: template?.positions(for: orientation) ?? [:],
                scale: appearance.scale)
        ) {
            LabeledContent("布局模板") {
                if let template { Text(verbatim: template.name) } else { Text("通用 Xbox") }
            }
            TouchAppearanceSlider(
                "透明度", value: $appearance.opacity,
                range: TouchLayoutAppearance.opacityRange, identifier: "touch-opacity")
            TouchAppearanceSlider(
                "大小", value: $appearance.scale,
                range: TouchLayoutAppearance.scaleRange, identifier: "touch-size")
            AppSettingsActionButton("恢复默认") { appearance = .init() }
                .accessibilityIdentifier("touch-appearance-reset")
            Text("调整会实时显示在预览中，保存后用于串流。")
                .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
        }
    }

    private func save() {
        settings.mobile.saveTouchAppearance(appearance, for: layoutID)
        dismiss()
    }
}

private struct TouchAppearanceSlider: View {
    let title: String
    @Binding var value: Double
    let range: ClosedRange<Double>
    let identifier: String
    init(_ title: String, value: Binding<Double>, range: ClosedRange<Double>, identifier: String) {
        self.title = title
        _value = value
        self.range = range
        self.identifier = identifier
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            LabeledContent {
                Text("\(Int((value * 100).rounded()))%")
                    .accessibilityIdentifier("\(identifier)-value")
            } label: {
                Text(LocalizedStringKey(title))
            }
            Slider(value: $value, in: range, step: 0.01)
                .frame(minHeight: 44)
                .accessibilityLabel(Text(LocalizedStringKey(title)))
                .accessibilityValue(Text("\(Int((value * 100).rounded()))%"))
                .accessibilityIdentifier(identifier)
        }
    }
}

@MainActor
private func orientationPicker(selection: Binding<TouchLayoutOrientation>) -> some View {
    AppSegmentedFilter(
        selection: selection, values: TouchLayoutOrientation.allCases,
        title: { $0.title }, identifier: "touch-preview-orientation", label: "预览方向",
        fillsWidth: false
    )
}

/// 画布占满实际窗口，工具浮在上方；收起设置后直接体验串流中的按键尺寸。
struct TouchLayoutWorkspace<SettingsContent: View>: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    let title: String
    let saveIdentifier: String
    let onSave: () -> Void
    @Binding var orientation: TouchLayoutOrientation
    let appearance: TouchLayoutAppearance
    let configuration: TouchLayoutConfiguration
    var onMove: ((String, CGPoint, CGSize, TouchSurfaceInsets) -> Void)? = nil
    var preparedLayout: PreparedTouchLayout? = nil
    var onBack: (() -> Void)? = nil
    var backIdentifier = "touch-workspace-back"
    var backLabel = "返回"
    var saving = false
    var saveDisabled = false
    var backDisabled = false
    @ViewBuilder let settingsContent: () -> SettingsContent
    @State private var showsSettings = false
    @State private var settingsHeight: CGFloat = 280
    @State private var toolsHeight: CGFloat = 100

    private var canSelectOrientation: Bool {
        UIDevice.current.userInterfaceIdiom == .phone
    }

    var body: some View {
        GeometryReader { geometry in
            ZStack(alignment: .top) {
                TouchLayoutCanvas(
                    orientation: $orientation, appearance: appearance,
                    configuration: configuration, settingsPresented: showsSettings || saving,
                    onMove: onMove, preparedLayout: preparedLayout
                )
                .ignoresSafeArea()
                .allowsHitTesting(!saving)
                VStack(spacing: 12) {
                    VStack(spacing: 12) {
                        workspaceTools
                        if !canSelectOrientation {
                            Text("旋转设备或调整窗口以切换方向")
                                .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .accessibilityIdentifier("touch-workspace-orientation-guidance")
                        }
                    }
                    .onGeometryChange(for: CGFloat.self) {
                        $0.size.height
                    } action: {
                        toolsHeight = $0
                    }
                    if showsSettings {
                        ScrollView {
                            VStack(alignment: .leading, spacing: 12, content: settingsContent)
                                .padding(16)
                                .onGeometryChange(for: CGFloat.self) {
                                    $0.size.height
                                } action: {
                                    settingsHeight = $0
                                }
                        }
                        .frame(
                            height: min(
                                settingsHeight, max(44, geometry.size.height - toolsHeight - 20))
                        )
                        .scrollBounceBehavior(.basedOnSize)
                        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16))
                        .overlay {
                            RoundedRectangle(cornerRadius: 16)
                                .strokeBorder(AppThemePalette.border, lineWidth: 1)
                        }
                        .accessibilityIdentifier("touch-workspace-settings-panel")
                    }
                }
                .frame(maxWidth: 420)
                .padding(.horizontal, 16).padding(.top, 8)
                .frame(maxWidth: .infinity)
            }
        }
        .ignoresSafeArea(.keyboard)
        .toolbar(.hidden, for: .navigationBar)
        .environment(\.colorScheme, .dark)
        .preferredColorScheme(.dark)
        .appResponsiveTypography()
        .appFont(.body).foregroundStyle(AppThemePalette.textPrimary)
        .tint(AppThemePalette.brand)
    }

    @ViewBuilder
    private var workspaceTools: some View {
        // 大字号独立呈现标题与方向胶囊，保证窄窗仍有完整的阅读和操作空间。
        if dynamicTypeSize.isAccessibilitySize {
            workspaceTitle
            HStack(spacing: 12) {
                backButton
                Spacer(minLength: 0)
                settingsButton
                saveButton
            }
            orientationControl.frame(maxWidth: .infinity, alignment: .leading)
        } else {
            HStack(spacing: 8) {
                backButton
                workspaceTitle
                saveButton
            }
            HStack(spacing: 12) {
                orientationControl
                Spacer(minLength: 0)
                settingsButton
            }
        }
    }

    private var workspaceTitle: some View {
        Text(LocalizedStringKey(title)).appFont(.title, weight: .semibold)
            .lineLimit(dynamicTypeSize.isAccessibilitySize ? nil : 1)
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityAddTraits(.isHeader)
            .accessibilityIdentifier("current-page-title")
    }

    private var backButton: some View {
        Button {
            if let onBack { onBack() } else { dismiss() }
        } label: {
            AppIconView(.chevron, size: 18).rotationEffect(.degrees(180))
                .frame(width: 44, height: 44)
        }
        .buttonStyle(.plain).disabled(saving || backDisabled)
        .accessibilityLabel(Text(LocalizedStringKey(backLabel)))
        .accessibilityIdentifier(backIdentifier)
    }

    private var saveButton: some View {
        Button(action: onSave) {
            if saving { ProgressView() } else { Text("保存") }
        }
        .buttonStyle(AppContentCapsuleButtonStyle())
        .disabled(saving || saveDisabled)
        .accessibilityIdentifier(saveIdentifier)
    }

    private var orientationControl: some View {
        orientationPicker(
            selection: Binding(
                get: { orientation },
                set: {
                    AppOrientationCoordinator.shared.requestInterfaceOrientation(
                        $0 == .portrait ? .portrait : .landscape)
                })
        )
        .disabled(!canSelectOrientation)
    }

    private var settingsButton: some View {
        Button {
            showsSettings.toggle()
        } label: {
            AppIconView(.sliders, size: 20).frame(width: 44, height: 44)
        }
        .buttonStyle(.plain)
        .appFilterTrackSurface()
        .accessibilityLabel(Text(LocalizedStringKey(showsSettings ? "收起设置" : "展开设置")))
        .accessibilityValue(Text(LocalizedStringKey(showsSettings ? "已展开" : "已收起")))
        .accessibilityIdentifier("touch-workspace-settings")
    }
}

/// 正式 Surface 直接消费真实窗口尺寸，竖屏视口沿串流合同保持在下半区。
private struct TouchLayoutCanvas: View {
    @Binding var orientation: TouchLayoutOrientation
    let appearance: TouchLayoutAppearance
    let configuration: TouchLayoutConfiguration
    let settingsPresented: Bool
    var onMove: ((String, CGPoint, CGSize, TouchSurfaceInsets) -> Void)? = nil
    var preparedLayout: PreparedTouchLayout? = nil
    @State private var input = TouchInputAccumulator()
    @State private var loadError: String?

    var body: some View {
        GeometryReader { geometry in
            let size = geometry.size
            let actualOrientation = TouchLayoutOrientation(size: size)
            let frame = StreamingTouchViewportPolicy.frame(in: size)
            ZStack(alignment: .topLeading) {
                Color(white: 0.06)
                Text("游戏画面").font(.system(size: 18)).foregroundStyle(.white.opacity(0.25))
                    .position(
                        x: size.width / 2,
                        y: actualOrientation == .portrait ? size.height / 4 : size.height / 2)
                if actualOrientation == .portrait {
                    Rectangle().fill(.white.opacity(0.08)).frame(width: size.width, height: 1)
                        .position(x: size.width / 2, y: size.height / 2)
                }
                TouchLayoutSurface(
                    input: input, opacity: appearance.opacity,
                    assets: preparedLayout?.assets ?? .empty,
                    configuration: configuration, onMoveControl: onMove
                )
                .frame(width: frame.width, height: frame.height)
                .position(x: frame.midX, y: frame.midY)
                .accessibilityIdentifier(
                    preparedLayout == nil ? "touch-layout-preview" : "official-touch-full-preview"
                )
                .accessibilityValue(actualOrientation.rawValue)
                if let loadError { Text(verbatim: loadError).foregroundStyle(.white).padding() }
            }
        }
        .onGeometryChange(for: TouchLayoutOrientation.self) {
            TouchLayoutOrientation(size: $0.size)
        } action: {
            orientation = $0
        }
        .onAppear {
            do { input.load(try preparedLayout?.layout ?? TouchLayoutCatalog.standard()) } catch {
                loadError = error.localizedDescription
            }
        }
        .onChange(of: orientation) { _, _ in input.cancelAll(reason: "previewOrientationChanged") }
        .onChange(of: settingsPresented) { _, _ in input.cancelAll(reason: "previewSettingsChanged")
        }
        .onDisappear { input.cancelAll(reason: "previewClosed") }
    }
}
