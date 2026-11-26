import SwiftUI
import UIKit

/// 输入按实际时间释放；短按的视觉反馈保留到用户能看见的下一帧。
struct TouchPressFeedback {
    static let minimumDuration: TimeInterval = 0.14
    private var deadlines: [String: TimeInterval] = [:]
    private var active: Set<String> = []

    mutating func update(_ frame: TouchInputFrame?, now: TimeInterval) -> Set<String> {
        guard let frame, frame.reason == "pointer" else {
            deadlines.removeAll()
            active.removeAll()
            return []
        }
        let next = Set(frame.activeControlIDs)
        let began = Set(frame.events.filter { $0.phase == .began }.compactMap(\.controlID))
        let pressed = next.subtracting(active).union(began)
        for id in pressed { deadlines[id] = now + Self.minimumDuration }
        for event in frame.events where event.phase == .cancelled {
            if let id = event.controlID, !next.contains(id) { deadlines.removeValue(forKey: id) }
        }
        active = next
        return pressed
    }

    func isHighlighted(_ id: String, now: TimeInterval) -> Bool {
        active.contains(id) || (deadlines[id] ?? 0) > now
    }

    mutating func needsDisplayFrame(now: TimeInterval) -> Bool {
        deadlines = deadlines.filter { $0.value > now }
        return deadlines.keys.contains { !active.contains($0) }
    }
}

struct TouchLayoutSurface: UIViewRepresentable {
    @Environment(\.locale) private var locale
    let input: TouchInputAccumulator
    let opacity: Double
    var assets: TouchLayoutAssets = .empty
    var capturesEmptySpace = true
    var configuration: TouchLayoutConfiguration? = nil
    var onMoveControl: ((String, CGPoint, CGSize, TouchSurfaceInsets) -> Void)? = nil
    var onEditingChanged: ((Bool) -> Void)? = nil
    var onPress: (() -> Void)? = nil

    func makeUIView(context: Context) -> TouchLayoutSurfaceView {
        TouchLayoutSurfaceView(input: input)
    }

    func updateUIView(_ view: TouchLayoutSurfaceView, context: Context) {
        view.locale = locale
        view.controlOpacity = Float(opacity)
        view.assets = assets
        view.capturesEmptySpace = capturesEmptySpace
        view.onMoveControl = onMoveControl
        view.onEditingChanged = onEditingChanged
        view.onPress = onPress
        input.setConfiguration(configuration)
        view.updateViewport()
        view.requestRender()
    }

    static func dismantleUIView(_ view: TouchLayoutSurfaceView, coordinator: ()) {
        view.stop()
    }
}

/// 一个触摸面保存所有指针，输入立即采集，绘制合并到下一个显示帧。
@MainActor
final class TouchLayoutSurfaceView: UIView {
    let input: TouchInputAccumulator
    var locale = Locale.current
    var controlOpacity: Float = 0.65
    var assets: TouchLayoutAssets = .empty
    var capturesEmptySpace = true
    var onMoveControl: ((String, CGPoint, CGSize, TouchSurfaceInsets) -> Void)?
    var onEditingChanged: ((Bool) -> Void)?
    var onPress: (() -> Void)?
    private var draggedControl: (pointer: ObjectIdentifier, id: String, offset: CGPoint)?
    private var selectedControlID: String?
    private var viewportSize = CGSize.zero
    private(set) var viewportInsets = TouchSurfaceInsets()
    private var feedback = TouchPressFeedback()
    private var identifiers: [ObjectIdentifier: Int] = [:]
    private var nextPointerID = 0
    private var drawings: [String: TouchControlDrawing] = [:]
    private var displayLink: CADisplayLink?
    private var displayTarget: TouchDisplayTarget?
    private var backgroundObserver: NSObjectProtocol?
    private var rendering = false

    init(input: TouchInputAccumulator) {
        self.input = input
        super.init(frame: .zero)
        isMultipleTouchEnabled = true
        isOpaque = false
        backgroundColor = .clear
        accessibilityIdentifier = "touch-layout-surface"
        input.onVisualChange = { [weak self] frame in
            guard let self else { return }
            let pressed = self.feedback.update(frame, now: CACurrentMediaTime())
            if frame?.reason != "pointer" { self.identifiers.removeAll() }
            if !pressed.isEmpty { self.onPress?() }
            self.requestRender()
        }
        backgroundObserver = NotificationCenter.default.addObserver(
            forName: UIApplication.willResignActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.cancel(reason: "background") }
        }
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window == nil {
            stop()
        } else {
            let target = TouchDisplayTarget()
            target.view = self
            displayTarget = target
            displayLink = CADisplayLink(
                target: target, selector: #selector(TouchDisplayTarget.tick))
            displayLink?.add(to: .main, forMode: .common)
            updateViewport()
            requestRender()
        }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        updateViewport()
        requestRender()
    }

    override func safeAreaInsetsDidChange() {
        super.safeAreaInsetsDidChange()
        updateViewport()
        requestRender()
    }

    func updateViewport() {
        let insets: TouchSurfaceInsets
        if let window {
            // 全屏画布保留原始尺寸；把窗口安全区投影到触摸面的局部坐标。
            let safe = convert(window.bounds.inset(by: window.safeAreaInsets), from: window)
            insets = TouchSurfaceInsets(
                top: max(0, safe.minY - bounds.minY),
                left: max(0, safe.minX - bounds.minX),
                bottom: max(0, bounds.maxY - safe.maxY),
                right: max(0, bounds.maxX - safe.maxX))
        } else {
            insets = .init()
        }
        guard viewportSize != bounds.size || viewportInsets != insets else { return }
        if draggedControl != nil {
            draggedControl = nil
            selectedControlID = nil
            onEditingChanged?(false)
        }
        viewportSize = bounds.size
        viewportInsets = insets
        input.setViewport(size: bounds.size, insets: insets)
    }

    func requestRender() {
        if displayLink == nil {
            if !rendering { render() }
        } else {
            displayLink?.isPaused = false
        }
    }

    override func point(inside point: CGPoint, with event: UIEvent?) -> Bool {
        guard super.point(inside: point, with: event) else { return false }
        return capturesEmptySpace
            || input.sockets.contains {
                $0.control.enabled && $0.contains(point)
            }
    }

    fileprivate func render() {
        guard !rendering else { return }
        rendering = true
        defer { rendering = false }
        let now = CACurrentMediaTime()
        input.advance(at: now)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let valid = Set(input.sockets.map(\.id))
        for key in drawings.keys where !valid.contains(key) {
            drawings.removeValue(forKey: key)?.root.removeFromSuperlayer()
        }
        var elements: [UIAccessibilityElement] = []
        for socket in input.sockets {
            let drawing = drawings[socket.id] ?? TouchControlDrawing()
            if drawings[socket.id] == nil {
                drawings[socket.id] = drawing
                layer.addSublayer(drawing.root)
            }
            drawing.update(
                socket: socket, runtime: input.runtimes[socket.id],
                opacity: controlOpacity, traits: traitCollection,
                assets: assets,
                highlighted: feedback.isHighlighted(socket.id, now: now)
                    || (onMoveControl != nil && selectedControlID == socket.id))
            let arcade = socket.control.kind == .arcadeButtons
            let accessibleSockets = arcade ? TouchArcadeGeometry.buttons(in: socket) : [socket]
            let activeButtons = arcade ? input.runtimes[socket.id]?.activeArcadeButtonIDs ?? [] : []
            for accessible in accessibleSockets {
                let element = UIAccessibilityElement(accessibilityContainer: self)
                element.accessibilityIdentifier = "touch-control-\(accessible.id)"
                element.accessibilityLabel = accessible.control.title
                element.accessibilityTraits = .button
                let active =
                    arcade
                    ? activeButtons.contains(accessible.id)
                    : input.runtimes[socket.id]?.isActive == true
                element.accessibilityValue = appLocalizedString(
                    onMoveControl != nil
                        ? (selectedControlID == socket.id ? "已选中，可拖动" : "可拖动")
                        : (active ? "按下" : "释放"), locale: locale)
                element.accessibilityFrameInContainerSpace = CGRect(
                    x: accessible.center.x - accessible.collisionRadius,
                    y: accessible.center.y - accessible.collisionRadius,
                    width: accessible.collisionRadius * 2, height: accessible.collisionRadius * 2)
                elements.append(element)
            }
        }
        accessibilityElements = elements
        CATransaction.commit()
        input.didRender(at: now)
        displayLink?.isPaused = !feedback.needsDisplayFrame(now: now) && !input.needsTimeUpdate
    }

    func cancel(reason: String) {
        identifiers.removeAll()
        draggedControl = nil
        selectedControlID = nil
        onEditingChanged?(false)
        input.cancelAll(reason: reason)
    }

    func stop() {
        cancel(reason: "surfaceClosed")
        displayLink?.invalidate()
        displayLink = nil
        displayTarget = nil
        if let backgroundObserver {
            NotificationCenter.default.removeObserver(backgroundObserver)
            self.backgroundObserver = nil
        }
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        sample(touches, phase: .began, event: event)
    }
    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) {
        sample(touches, phase: .moved, event: event)
    }
    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        sample(touches, phase: .ended, event: event)
    }
    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        sample(touches, phase: .cancelled, event: event)
    }

    private func sample(_ touches: Set<UITouch>, phase: TouchPointerPhase, event: UIEvent?) {
        if onMoveControl != nil {
            edit(touches, phase: phase)
            return
        }
        let receivedAt = ProcessInfo.processInfo.systemUptime
        var samples: [TouchPointerSample] = []
        for touch in touches.sorted(by: { $0.timestamp < $1.timestamp }) {
            let identity = ObjectIdentifier(touch)
            if phase == .began {
                nextPointerID += 1
                identifiers[identity] = nextPointerID
            }
            guard let id = identifiers[identity] else { continue }
            func point(_ source: UITouch) -> TouchPointerSample {
                .init(
                    id: id, phase: phase, position: source.location(in: self),
                    pressure: source.maximumPossibleForce > 0
                        ? Double(source.force / source.maximumPossibleForce) : 0,
                    sourceTimestamp: source.timestamp)
            }
            let history = phase == .began ? [] : (event?.coalescedTouches(for: touch) ?? [])
            samples.append(
                contentsOf: TouchSampleBatch.coalescing(
                    current: point(touch), history: history.map(point)))
            if phase == .ended || phase == .cancelled { identifiers.removeValue(forKey: identity) }
        }
        input.process(TouchSampleBatch.ordered(samples), timestamp: receivedAt)
    }

    private func edit(_ touches: Set<UITouch>, phase: TouchPointerPhase) {
        for touch in touches.sorted(by: { $0.timestamp < $1.timestamp }) {
            let pointer = ObjectIdentifier(touch)
            let point = touch.location(in: self)
            if phase == .began, draggedControl == nil,
                let socket = TouchLayoutGeometry.hitTest(point, sockets: input.sockets)
            {
                selectedControlID = socket.id
                draggedControl = (
                    pointer, socket.id,
                    CGPoint(x: socket.center.x - point.x, y: socket.center.y - point.y)
                )
                onEditingChanged?(true)
                requestRender()
            }
            guard let drag = draggedControl, drag.pointer == pointer else { continue }
            if phase == .moved || phase == .ended {
                onMoveControl?(
                    drag.id,
                    CGPoint(x: point.x + drag.offset.x, y: point.y + drag.offset.y), bounds.size,
                    viewportInsets)
            }
            if phase == .ended || phase == .cancelled {
                draggedControl = nil
                onEditingChanged?(false)
            }
        }
    }
}

@MainActor
private final class TouchDisplayTarget: NSObject {
    weak var view: TouchLayoutSurfaceView?
    @objc func tick() { view?.render() }
}

@MainActor
private final class TouchControlDrawing {
    let root = CALayer()
    private let outline = CAShapeLayer()
    private let indicator = CAShapeLayer()
    private let knob = CAShapeLayer()
    private let symbol = CALayer()
    private let text = CATextLayer()
    private let background = CALayer()
    private let knobBackground = CALayer()
    private let axisUpCap = CALayer()
    private let axisDownCap = CALayer()
    private let pullConnector = CAShapeLayer()
    private var symbolCacheKey: String?
    private var symbolCache: CGImage?
    private var arcadeDrawings: [String: TouchControlDrawing] = [:]
    private var pullDrawings: [Int: TouchControlDrawing] = [:]
    private var previousArcadeActiveIDs: Set<String> = []

    init() {
        for child in [outline, indicator, knob, symbol, text] { root.addSublayer(child) }
        // 追加素材层并用深度排序，保持既有控件层和资源复用合同。
        background.name = "control-background"
        background.zPosition = -1
        knobBackground.name = "knob-background"
        knobBackground.zPosition = 0.5
        axisUpCap.name = "axis-up-cap"
        axisDownCap.name = "axis-down-cap"
        for child in [background, knobBackground, axisUpCap, axisDownCap, pullConnector] {
            root.addSublayer(child)
        }
        knob.zPosition = 1
        symbol.zPosition = 2
        text.zPosition = 2
        pullConnector.zPosition = 1.5
        pullConnector.lineWidth = 1.3
        pullConnector.fillColor = UIColor.clear.cgColor
        for shape in [outline, indicator, knob] {
            shape.lineWidth = 1.3
            shape.lineCap = .round
            shape.lineJoin = .round
        }
        text.alignmentMode = .center
        text.font = UIFont.systemFont(ofSize: 16, weight: .semibold)
    }

    func update(
        socket: TouchControlSocket, runtime: TouchControlRuntime?, opacity: Float,
        traits: UITraitCollection, assets: TouchLayoutAssets, highlighted: Bool
    ) {
        if socket.control.kind == .arcadeButtons {
            updateArcade(
                socket: socket, runtime: runtime, opacity: opacity, traits: traits,
                assets: assets, highlighted: highlighted)
            return
        }
        for drawing in arcadeDrawings.values { drawing.root.removeFromSuperlayer() }
        arcadeDrawings.removeAll()
        outline.isHidden = false
        symbol.isHidden = false
        let diameter = socket.diameter
        let bounds = CGRect(x: 0, y: 0, width: diameter, height: diameter)
        let active = runtime?.isActive == true
        let pressed = socket.control.enabled && (active || highlighted)
        root.bounds = bounds
        root.position = socket.center
        root.transform = CATransform3DMakeScale(pressed ? 0.94 : 1, pressed ? 0.94 : 1, 1)
        let foreground = UIColor.white
        let accent = UIColor(red: 0.31, green: 0.69, blue: 0.48, alpha: 1)
        let style = socket.control.visualStyle(runtime: runtime)
        root.opacity = (pressed ? max(opacity, 0.85) : opacity) * styleOpacity(style)
        if !socket.control.enabled
            && socket.control.properties["styles"]?["disabled"]?["opacity"] == nil
        {
            root.opacity *= 0.5
        }
        let stroke = style["knob"]?["stroke"] ?? style["stroke"]
        let strokeColor = color(stroke?["color"]?.string) ?? (pressed ? accent : foreground)
        let feedbackColor = buttonColor(socket.control.action, fallback: strokeColor)
        outline.frame = bounds
        let isTouchpad = socket.control.kind == .touchpad && !socket.control.renderAsButton
        updateArtwork(
            background, component: style["background"], frame: bounds,
            cornerRadius: diameter * (isTouchpad ? 0.18 : 0.5), traits: traits, assets: assets)
        outline.path =
            isTouchpad
            ? UIBezierPath(
                roundedRect: bounds.insetBy(dx: 1, dy: 1), cornerRadius: diameter * 0.18
            ).cgPath
            : UIBezierPath(ovalIn: bounds.insetBy(dx: 1, dy: 1)).cgPath
        outline.fillColor =
            background.isHidden
            ? (pressed ? feedbackColor : foreground)
                .withAlphaComponent(pressed ? 0.22 : 0.025).cgColor
            : UIColor.clear.cgColor
        outline.strokeColor =
            (pressed ? feedbackColor : foreground)
            .withAlphaComponent(pressed ? 0.95 : 0.35).cgColor
        outline.lineWidth = pressed ? 2.4 : 1.3
        outline.opacity = pressed ? 1 : Float(style["outline"]?["opacity"]?.number ?? 1)
        knob.isHidden = true
        knobBackground.isHidden = true
        knobBackground.contents = nil
        axisUpCap.isHidden = true
        axisDownCap.isHidden = true
        indicator.isHidden = true

        let kind = socket.control.kind
        var face = bounds.insetBy(dx: diameter * 0.21, dy: diameter * 0.21)
        if kind == .joystick || kind == .throttle {
            let value = runtime?.value ?? .zero
            let offset = CGPoint(x: value.x * diameter * 0.23, y: -value.y * diameter * 0.23)
            face = CGRect(
                x: diameter * 0.27 + offset.x, y: diameter * 0.27 + offset.y,
                width: diameter * 0.46, height: diameter * 0.46)
            updateArtwork(
                knobBackground, component: style["knob"]?["background"], frame: face,
                cornerRadius: face.width / 2, traits: traits, assets: assets)
            knobBackground.opacity *= styleOpacity(style["knob"])
            knob.isHidden = false
            knob.frame = bounds
            knob.path = UIBezierPath(ovalIn: face).cgPath
            knob.strokeColor = strokeColor.cgColor
            let knobOpacity = styleOpacity(stroke, fallback: 0.8)
            knob.opacity =
                (pressed ? max(knobOpacity, 0.9) : knobOpacity) * styleOpacity(style["knob"])
            knob.fillColor =
                knobBackground.isHidden
                ? (pressed ? feedbackColor : foreground)
                    .withAlphaComponent(pressed ? 0.18 : 0.045).cgColor
                : UIColor.clear.cgColor
            if kind == .throttle {
                indicator.isHidden = false
                let path = UIBezierPath()
                path.move(to: CGPoint(x: diameter / 2, y: diameter * 0.13))
                path.addLine(to: CGPoint(x: diameter / 2, y: diameter * 0.87))
                indicator.frame = bounds
                indicator.path = path.cgPath
                indicator.strokeColor =
                    (color(style["indicator"]?["color"]?.string) ?? accent)
                    .withAlphaComponent(0.55).cgColor
                for (cap, key, y) in [(axisUpCap, "axisUp", 0.13), (axisDownCap, "axisDown", 0.87)]
                {
                    updateArtwork(
                        cap, component: style[key]?["cap"],
                        frame: CGRect(
                            x: diameter * 0.375, y: diameter * (y - 0.04),
                            width: diameter * 0.25, height: diameter * 0.08),
                        cornerRadius: diameter * 0.04, traits: traits, assets: assets)
                }
            }
            face = face.insetBy(dx: face.width * 0.22, dy: face.height * 0.22)
        }
        if kind == .directionalPad {
            let radius = socket.collisionRadius * socket.control.directionalPadDeadzone
            indicator.isHidden = radius == 0
            indicator.frame = bounds
            indicator.path =
                UIBezierPath(
                    ovalIn: CGRect(
                        x: diameter / 2 - radius, y: diameter / 2 - radius,
                        width: radius * 2, height: radius * 2)
                ).cgPath
            indicator.strokeColor = foreground.withAlphaComponent(0.25).cgColor
            indicator.fillColor = UIColor.clear.cgColor
        }
        let faceImage = style["faceImage"] ?? style["knob"]?["faceImage"]
        let icon = faceImage?["value"]?.string ?? socket.control.icon
        let asset = faceImage?["type"]?.string == "asset" ? icon.flatMap { assets.images[$0] } : nil
        let systemName = symbolName(icon: icon, control: socket.control)
        symbol.frame = face
        symbol.contentsScale = traits.displayScale
        symbol.contentsGravity = .resizeAspect
        symbol.opacity = styleOpacity(faceImage)
        if style["faceImage"] == nil { symbol.opacity *= styleOpacity(style["knob"]) }
        let isXboxGuide = socket.control.action == .gamepad("guide")
        let symbolIdentity = isXboxGuide ? "XboxLogo" : systemName ?? ""
        let cacheKey = "\(symbolIdentity): \(face.size): \(strokeColor): \(traits.displayScale)"
        if asset == nil, cacheKey != symbolCacheKey {
            symbolCacheKey = cacheKey
            let artwork =
                isXboxGuide
                ? UIImage(named: "XboxLogo")
                : systemName.flatMap { name in
                    UIImage(
                        systemName: name,
                        withConfiguration: UIImage.SymbolConfiguration(
                            pointSize: max(18, face.width), weight: .regular))
                }
            symbolCache = artwork.flatMap { image in
                let format = UIGraphicsImageRendererFormat()
                format.opaque = false
                format.scale = traits.displayScale
                return UIGraphicsImageRenderer(size: face.size, format: format).image { _ in
                    image.withTintColor(strokeColor, renderingMode: .alwaysOriginal)
                        .draw(in: CGRect(origin: .zero, size: face.size))
                }.cgImage
            }
        }
        symbol.contents = asset ?? symbolCache
        text.isHidden = asset != nil || systemName != nil
        text.contentsScale = traits.displayScale
        text.frame = CGRect(
            x: 0, y: diameter / 2 - min(20, diameter * 0.38) * 0.62,
            width: diameter, height: diameter * 0.6)
        text.fontSize = min(20, diameter * 0.38)
        text.foregroundColor = (pressed ? foreground : feedbackColor).cgColor
        text.string = socket.control.action?.title ?? ""
        if kind == .joystick {
            text.frame = CGRect(
                x: face.minX, y: face.midY - face.height * 0.38,
                width: face.width, height: face.height)
            text.fontSize = face.height * 0.6
            text.string = socket.control.axis.contains { $0.output.hasPrefix("left") } ? "L" : "R"
        }
        if isXboxGuide {
            text.isHidden = true
            indicator.isHidden = true
        }
        updatePullIndicators(
            socket: socket, runtime: runtime, style: style, traits: traits, assets: assets)
    }

    private func updateArtwork(
        _ layer: CALayer, component: TouchJSON?, frame: CGRect, cornerRadius: CGFloat,
        traits: UITraitCollection, assets: TouchLayoutAssets
    ) {
        layer.frame = frame
        layer.contentsScale = traits.displayScale
        layer.contentsGravity = .resizeAspect
        layer.opacity = styleOpacity(component)
        layer.contents = nil
        layer.backgroundColor = nil
        layer.cornerRadius = 0
        layer.masksToBounds = false
        if component?["type"]?.string == "asset", let id = component?["value"]?.string {
            layer.contents = assets.images[id]
        } else if component?["type"]?.string == "color",
            let fill = color(component?["value"]?.string)
        {
            layer.backgroundColor = fill.cgColor
            layer.cornerRadius = cornerRadius
            layer.masksToBounds = true
        }
        layer.isHidden = layer.contents == nil && layer.backgroundColor == nil
    }

    private func updatePullIndicators(
        socket: TouchControlSocket, runtime: TouchControlRuntime?, style: TouchJSON,
        traits: UITraitCollection, assets: TouchLayoutAssets
    ) {
        let pulls = socket.control.enabled ? runtime?.pullStates ?? [] : []
        let valid = Set(pulls.map(\.pointerID))
        for id in pullDrawings.keys where !valid.contains(id) {
            pullDrawings.removeValue(forKey: id)?.root.removeFromSuperlayer()
        }
        pullConnector.isHidden = pulls.isEmpty
        let path = UIBezierPath()
        let center = CGPoint(x: socket.diameter / 2, y: socket.diameter / 2)
        for pull in pulls {
            let length = hypot(pull.offset.x, pull.offset.y)
            let distance = socket.diameter * 0.7
            let position = CGPoint(
                x: center.x + pull.offset.x / length * distance,
                y: center.y + pull.offset.y / length * distance)
            path.move(to: center)
            path.addLine(to: position)
            let segment: TouchJSON?
            let action: TouchJSON?
            if let index = pull.segmentIndex {
                let segments = style["pullIndicator"]?.array
                segment =
                    segments.flatMap { $0.indices.contains(index) ? $0[index] : nil }
                    ?? (segments == nil ? style["pullIndicator"] : nil)
                action = socket.control.properties["pullAction"]?["items"]?.array?[index]
            } else {
                segment = style["pullIndicator"]
                action = socket.control.properties["pullAction"]
            }
            let control = TouchControl(
                id: "\(socket.id)/pull/\(pull.pointerID)", kind: .button,
                properties: .object([
                    "action": action ?? .null,
                    "styles": .object(["default": segment ?? .object([:])]),
                ]))
            let local = TouchControlSocket(
                control: control, center: position, diameter: socket.diameter * 0.42,
                unitSize: socket.unitSize, regionOrder: socket.regionOrder)
            let drawing = pullDrawings[pull.pointerID] ?? TouchControlDrawing()
            if pullDrawings[pull.pointerID] == nil {
                pullDrawings[pull.pointerID] = drawing
                drawing.root.name = "pull-indicator-\(pull.pointerID)"
                drawing.root.zPosition = 3
                root.addSublayer(drawing.root)
            }
            drawing.update(
                socket: local, runtime: nil, opacity: 1, traits: traits, assets: assets,
                highlighted: false)
            drawing.root.isHidden = segment == .null
        }
        pullConnector.frame = root.bounds
        pullConnector.path = path.cgPath
        pullConnector.strokeColor = UIColor.white.withAlphaComponent(0.45).cgColor
    }

    private func updateArcade(
        socket: TouchControlSocket, runtime: TouchControlRuntime?, opacity: Float,
        traits: UITraitCollection, assets: TouchLayoutAssets, highlighted: Bool
    ) {
        let diameter = socket.diameter
        root.bounds = CGRect(x: 0, y: 0, width: diameter, height: diameter)
        root.position = socket.center
        root.transform = CATransform3DIdentity
        root.opacity = 1
        for child in [
            outline, indicator, knob, symbol, text, background, knobBackground, axisUpCap,
            axisDownCap, pullConnector,
        ] {
            child.isHidden = true
            child.contents = nil
        }
        for drawing in pullDrawings.values { drawing.root.removeFromSuperlayer() }
        pullDrawings.removeAll()
        let active = runtime?.activeArcadeButtonIDs ?? []
        if !active.isEmpty || runtime?.pointers.isEmpty == false {
            previousArcadeActiveIDs = active
        }
        let displayed = active.isEmpty && highlighted ? previousArcadeActiveIDs : active
        let buttons = TouchArcadeGeometry.buttons(in: socket)
        let valid = Set(buttons.map(\.id))
        for id in arcadeDrawings.keys where !valid.contains(id) {
            arcadeDrawings.removeValue(forKey: id)?.root.removeFromSuperlayer()
        }
        for button in buttons {
            let local = TouchControlSocket(
                control: button.control,
                center: CGPoint(
                    x: button.center.x - socket.center.x + diameter / 2,
                    y: button.center.y - socket.center.y + diameter / 2),
                diameter: button.diameter, unitSize: button.unitSize,
                regionOrder: button.regionOrder)
            let drawing = arcadeDrawings[button.id] ?? TouchControlDrawing()
            if arcadeDrawings[button.id] == nil {
                arcadeDrawings[button.id] = drawing
                root.addSublayer(drawing.root)
            }
            var buttonRuntime = TouchControlRuntime(socket: local)
            if displayed.contains(button.id) {
                buttonRuntime.begin(pointerID: 0, position: local.center)
            }
            drawing.update(
                socket: local, runtime: buttonRuntime, opacity: opacity, traits: traits,
                assets: assets,
                highlighted: displayed.contains(button.id))
        }
        if !highlighted && active.isEmpty { previousArcadeActiveIDs.removeAll() }
    }

    private func symbolName(icon: String?, control: TouchControl) -> String? {
        let symbols = [
            "steering": "steeringwheel", "switchCamera": "camera.rotate", "dPad": "dpad",
            "gasPedal": "pedal.accelerator", "brakePedal": "pedal.brake",
            "handbrake2": "parkingsign.circle",
            "add": "plus", "subtract": "minus", "repeatRefresh": "arrow.counterclockwise",
            "jump": "arrow.up", "crouch": "arrow.down", "fire": "scope", "aim": "scope",
            "reload": "arrow.clockwise",
        ]
        if let icon, let symbol = symbols[icon], UIImage(systemName: symbol) != nil {
            return symbol
        }
        if control.kind == .directionalPad { return "dpad" }
        if control.action == .gamepad("menu") { return "line.3.horizontal" }
        if control.action == .gamepad("view") { return "rectangle.on.rectangle" }
        if icon == "gasPedal" { return "arrow.up.to.line" }
        if icon == "brakePedal" { return "arrow.down.to.line" }
        return nil
    }

    private func buttonColor(_ action: TouchAction?, fallback: UIColor) -> UIColor {
        switch action {
        case .gamepad("gamepadA"): UIColor(red: 0.38, green: 0.73, blue: 0.46, alpha: 1)
        case .gamepad("gamepadB"): UIColor(red: 0.86, green: 0.42, blue: 0.42, alpha: 1)
        case .gamepad("gamepadX"): UIColor(red: 0.38, green: 0.62, blue: 0.85, alpha: 1)
        case .gamepad("gamepadY"): UIColor(red: 0.85, green: 0.73, blue: 0.36, alpha: 1)
        default: fallback
        }
    }

    private func color(_ hex: String?) -> UIColor? {
        guard let hex, hex.hasPrefix("#") else { return nil }
        var digits = String(hex.dropFirst())
        if digits.count == 3 || digits.count == 4 {
            digits = String(digits.flatMap { [$0, $0] })
        }
        guard (digits.count == 6 || digits.count == 8), let value = UInt32(digits, radix: 16) else {
            return nil
        }
        let rgb = digits.count == 8 ? value >> 8 : value
        return UIColor(
            red: CGFloat((rgb >> 16) & 255) / 255,
            green: CGFloat((rgb >> 8) & 255) / 255,
            blue: CGFloat(rgb & 255) / 255,
            alpha: digits.count == 8 ? CGFloat(value & 255) / 255 : 1)
    }

    private func styleOpacity(_ style: TouchJSON?, fallback: Float = 1) -> Float {
        guard let opacity = style?["opacity"]?.number, opacity.isFinite else { return fallback }
        return Float(min(1, max(0, opacity)))
    }
}

extension TouchControl {
    func visualStyle(runtime: TouchControlRuntime?) -> TouchJSON {
        let styles = properties["styles"]
        let state: String
        if !enabled {
            state = "disabled"
        } else if kind == .throttle, let runtime {
            state =
                runtime.pointers.isEmpty
                ? (runtime.value.y > 0 ? "idleUp" : "idle")
                : (runtime.value.y >= 0 ? "activatedUp" : "activatedDown")
        } else if kind == .button, runtime?.isPulled == true {
            state = "pulled"
        } else if kind == .button, runtime?.toggled == true {
            state = "toggled"
        } else if kind == .joystick, let runtime, !runtime.pointers.isEmpty {
            state =
                action != nil && hypot(runtime.value.x, runtime.value.y) >= actionThreshold
                ? "activated" : "moving"
        } else if kind == .touchpad, runtime?.pointers.isEmpty == false {
            state = action != nil ? "activated" : "moving"
        } else {
            state = runtime?.isActive == true ? "activated" : "idle"
        }
        func merge(_ base: TouchJSON?, _ patch: TouchJSON?) -> TouchJSON {
            guard let patch else { return base ?? .object([:]) }
            guard let object = patch.object else { return patch }
            var result = base?.object ?? [:]
            for (key, value) in object { result[key] = merge(result[key], value) }
            return .object(result)
        }
        return merge(styles?["default"], styles?[state])
    }
}
