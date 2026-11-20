import SwiftUI

struct TouchLayoutEditorView: View {
    @EnvironmentObject private var settings: AppSettingsStore
    @Environment(\.dismiss) private var dismiss
    @State private var draft: TouchLayoutTemplate
    @State private var orientation: TouchLayoutOrientation = .landscape
    @State private var isDragging = false
    @State private var showDelete = false
    @State private var showLimit = false
    private let originalID: String?

    init(template: TouchLayoutTemplate?, appearance: TouchLayoutAppearance) {
        originalID = template?.id
        _draft = State(initialValue: template ?? TouchLayoutTemplate(appearance: appearance))
    }

    var body: some View {
        AppSettingsForm {
            Section("布局") {
                LabeledContent("模板名称") {
                    TextField("输入名称", text: $draft.name)
                        .multilineTextAlignment(.trailing)
                        .accessibilityIdentifier("touch-template-name")
                }
                orientationPicker(selection: $orientation)
                TouchLayoutPreview(
                    orientation: orientation, appearance: draft.appearance,
                    configuration: .init(
                        identity: draft.id, positions: draft.positions(for: orientation),
                        scale: draft.appearance.scale),
                    onMove: { id, point, size in
                        draft.setPosition(
                            TouchLayoutGeometry.normalizedPosition(point, size: size),
                            for: id, orientation: orientation)
                    }, onEditingChanged: { isDragging = $0 }
                )
                .appSettingsInformation()
                Text("拖动按键调整位置，横屏和竖屏分别保存。")
                    .appFont(.caption).foregroundStyle(AppThemePalette.textSecondary)
                    .appSettingsInformation()
                AppSettingsActionButton("恢复位置") { draft.resetPositions(for: orientation) }
                    .accessibilityIdentifier("touch-template-reset")
            }
            if originalID != nil {
                Section("本地模板") {
                    AppSettingsActionButton("另存为新模板") { save(asNew: true) }
                        .accessibilityIdentifier("touch-template-save-new")
                    AppSettingsActionButton("删除模板", role: .destructive) { showDelete = true }
                        .accessibilityIdentifier("touch-template-delete")
                }
            }
        }
        .scrollDisabled(isDragging)
        .appPageToolbar("自定义布局", showsBrand: false)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button("保存") { save() }.frame(minWidth: 44, minHeight: 44)
                    .accessibilityIdentifier("touch-template-save")
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
    @State private var orientation: TouchLayoutOrientation = .landscape
    private let template: TouchLayoutTemplate?
    private let layoutID: String

    init(template: TouchLayoutTemplate?, appearance: TouchLayoutAppearance) {
        self.template = template
        layoutID = template?.id ?? TouchLayoutTemplate.standardID
        _appearance = State(initialValue: appearance)
    }

    var body: some View {
        AppSettingsForm {
            Section("当前布局") {
                LabeledContent("布局模板") {
                    if let template { Text(verbatim: template.name) } else { Text("通用 Xbox") }
                }.appSettingsInformation()
                orientationPicker(selection: $orientation)
                TouchLayoutPreview(
                    orientation: orientation, appearance: appearance,
                    configuration: .init(
                        identity: layoutID,
                        positions: template?.positions(for: orientation) ?? [:],
                        scale: appearance.scale)
                )
                .appSettingsInformation()
            }
            Section("触屏手柄调整") {
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
                    .appSettingsInformation()
            }
        }
        .appPageToolbar("触屏手柄调整", showsBrand: false)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button("保存") {
                    settings.mobile.saveTouchAppearance(appearance, for: layoutID)
                    dismiss()
                }.frame(minWidth: 44, minHeight: 44)
                    .accessibilityIdentifier("touch-appearance-save")
            }
        }
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

private func orientationPicker(selection: Binding<TouchLayoutOrientation>) -> some View {
    Picker("预览方向", selection: selection) {
        ForEach(TouchLayoutOrientation.allCases) { Text(LocalizedStringKey($0.title)).tag($0) }
    }
    .pickerStyle(.segmented).frame(minHeight: 44)
    .accessibilityIdentifier("touch-preview-orientation")
}

/// 使用正式 Surface 按参考屏幕比例缩放，触点坐标由视图变换回到同一逻辑视口。
private struct TouchLayoutPreview: View {
    let orientation: TouchLayoutOrientation
    let appearance: TouchLayoutAppearance
    let configuration: TouchLayoutConfiguration
    var onMove: ((String, CGPoint, CGSize) -> Void)? = nil
    var onEditingChanged: ((Bool) -> Void)? = nil
    @State private var input = TouchInputAccumulator()
    @State private var loadError: String?

    var body: some View {
        GeometryReader { geometry in
            let size = orientation.previewSize
            let scale = geometry.size.width / size.width
            let frame = StreamingTouchViewportPolicy.frame(in: size)
            ZStack(alignment: .topLeading) {
                Color(white: 0.06)
                ZStack(alignment: .topLeading) {
                    Text("游戏画面").font(.system(size: 18)).foregroundStyle(.white.opacity(0.25))
                        .position(
                            x: size.width / 2,
                            y: orientation == .portrait ? size.height / 4 : size.height / 2)
                    if orientation == .portrait {
                        Rectangle().fill(.white.opacity(0.08)).frame(width: size.width, height: 1)
                            .position(x: size.width / 2, y: size.height / 2)
                    }
                    TouchLayoutSurface(
                        input: input, opacity: appearance.opacity,
                        configuration: configuration, onMoveControl: onMove,
                        onEditingChanged: onEditingChanged
                    )
                    .frame(width: frame.width, height: frame.height)
                    .position(x: frame.midX, y: frame.midY)
                }
                .frame(width: size.width, height: size.height)
                .scaleEffect(scale, anchor: .topLeading)
                if let loadError { Text(verbatim: loadError).foregroundStyle(.white).padding() }
            }
        }
        .aspectRatio(
            orientation.previewSize.width / orientation.previewSize.height, contentMode: .fit
        )
        .clipShape(RoundedRectangle(cornerRadius: 12))
        .accessibilityIdentifier("touch-layout-preview")
        .onAppear {
            do { input.load(try TouchLayoutCatalog.standard()) } catch {
                loadError = error.localizedDescription
            }
        }
        .onChange(of: orientation) { _, _ in input.cancelAll(reason: "previewOrientationChanged") }
    }
}
