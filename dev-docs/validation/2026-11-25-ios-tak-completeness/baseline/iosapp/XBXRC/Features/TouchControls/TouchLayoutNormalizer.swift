import Foundation

enum TouchLayoutNormalizer {
    static let supportedActions: Set<String> = [
        "gamepadA", "gamepadB", "gamepadX", "gamepadY", "menu", "view", "guide",
        "leftBumper", "rightBumper", "leftThumb", "rightThumb", "leftTrigger", "rightTrigger",
        "dPadUp", "dPadDown", "dPadLeft", "dPadRight",
        "leftJoystickUp", "leftJoystickDown", "leftJoystickLeft", "leftJoystickRight",
        "rightJoystickUp", "rightJoystickDown", "rightJoystickLeft", "rightJoystickRight",
    ]
    static let supportedOutputs: Set<String> = [
        "leftJoystick", "rightJoystick", "leftJoystickX", "leftJoystickY",
        "rightJoystickX", "rightJoystickY", "leftTrigger", "rightTrigger",
        "leftJoystickUp", "leftJoystickDown", "leftJoystickLeft", "leftJoystickRight",
        "rightJoystickUp", "rightJoystickDown", "rightJoystickLeft", "rightJoystickRight",
    ]
    static let supportedInputs: Set<String> = [
        "axisXY", "axisX", "axisY", "axisUp", "axisDown", "axisLeft", "axisRight",
    ]
    static let supportedIcons: Set<String> = [
        "steering", "switchCamera", "dPad", "gasPedal", "brakePedal", "handbrake2",
        "add", "subtract", "repeatRefresh", "jump", "crouch", "fire", "aim", "reload",
        "gamepadA", "gamepadB", "gamepadX", "gamepadY",
    ]

    static func decode(
        _ data: Data, defaultID: String = "layout", availableAssets: Set<String> = []
    ) throws -> [TouchLayout] {
        guard data.count <= 2_000_000 else { throw TouchLayoutError.invalid("文件超过 2 MB") }
        let root = try JSONDecoder().decode(TouchJSON.self, from: data)
        guard root.object != nil else { throw TouchLayoutError.invalid("根节点需要为对象") }
        if let layouts = root["layouts"]?.object {
            guard root["schema_version"]?.number == 1 else {
                throw TouchLayoutError.invalid("社区包装版本需要为 1")
            }
            return try layouts.keys.sorted().map {
                try normalize(layouts[$0]!, id: $0, format: "community", assets: availableAssets)
            }
        }
        return [try normalize(root, id: defaultID, format: "official", assets: availableAssets)]
    }

    private static func normalize(
        _ document: TouchJSON, id: String, format: String, assets: Set<String>
    ) throws
        -> TouchLayout
    {
        guard let content = document["content"], content.object != nil else {
            throw TouchLayoutError.invalid("\(id) 缺少 content")
        }
        var report = TouchCapabilityReport(
            format: format, schema: document["$schema"]?.string ?? "runtime-v1")
        if let schema = document["$schema"]?.string,
            !supportsSchema(schema)
        {
            report.add(.unsupported, "/$schema", "此布局版本等待转换器支持")
        }
        let value = normalizeNode(content, path: "/content", depth: 0, report: &report)
        scanContent(value, path: "/content", assets: assets, report: &report)
        validateLayerTargets(
            value, path: "/content", layers: Set(value["layers"]?.object?.keys.map { $0 } ?? []),
            report: &report)
        report.controlTypes = Array(Set(report.controlTypes)).sorted()
        report.behaviors = Array(Set(report.behaviors)).sorted()
        return TouchLayout(
            id: id, name: document["name"]?.string ?? document["displayName"]?.string ?? id,
            author: document["author"]?.string, content: value, report: report)
    }

    private static func supportsSchema(_ schema: String) -> Bool {
        guard let url = URL(string: schema),
            url.host == "schema.play.xbox.com"
                || (url.host == "raw.githubusercontent.com"
                    && url.path.hasPrefix("/microsoft/xbox-game-streaming-tools/"))
        else { return false }
        let parts = url.pathComponents
        guard let layout = parts.firstIndex(of: "layout"), parts.count > layout + 1 else {
            return false
        }
        let component = parts[layout + 1]
        let version = component.hasPrefix("v") ? String(component.dropFirst()) : component
        return ["latest", "1.0", "2.0", "3.0", "3.1", "3.2", "3.3", "3.4", "3.5", "4.0", "4.1"]
            .contains(version)
    }

    private static func normalizeNode(
        _ value: TouchJSON, path: String, depth: Int, report: inout TouchCapabilityReport
    ) -> TouchJSON {
        guard depth <= 32 else {
            report.add(.unsupported, path, "嵌套层级超过 32")
            return .null
        }
        if let array = value.array {
            return .array(
                array.enumerated().map {
                    normalizeNode(
                        $0.element, path: "\(path)/\($0.offset)", depth: depth + 1, report: &report)
                })
        }
        guard var object = value.object else { return value }
        if object["$ref"] != nil {
            report.add(.unsupported, path, "Context / definitions 引用等待资源解析")
        }
        for key in object.keys.sorted() {
            object[key] = normalizeNode(
                object[key]!, path: "\(path)/\(key)", depth: depth + 1, report: &report)
        }
        if let type = object["type"]?.string, TouchControlKind(rawValue: type) != nil {
            // 社区直接注入路径依赖这些 TAK 默认值，归一化后各宿主共用。
            for key in ["visible", "enabled"] where object[key] == nil { object[key] = .bool(true) }
            if type == "joystick" || type == "throttle" {
                if object["relative"] == nil { object["relative"] = .bool(true) }
            }
            if type == "joystick", object["expand"] == nil { object["expand"] = .bool(true) }
        }
        if path.hasSuffix("/stroke"), object["type"] == nil {
            object["type"] = .string("solid")
            report.add(.warning, path, "使用运行时默认描边类型 solid")
        }
        return .object(object)
    }

    private static func scanContent(
        _ content: TouchJSON, path: String, assets: Set<String>, report: inout TouchCapabilityReport
    ) {
        let regions: Set<String> = [
            "left", "right", "center", "upper", "lower", "layers", "sensors",
        ]
        for key in (content.object ?? [:]).keys.sorted() where !regions.contains(key) {
            report.add(.unsupported, "\(path)/\(key)", "此内容区域等待支持")
        }
        for side in ["left", "right", "center"] {
            guard let wheel = content[side] else { continue }
            guard wheel == .null || wheel.object != nil else {
                report.add(.unsupported, "\(path)/\(side)", "轮盘需要为对象")
                continue
            }
            for key in (wheel.object ?? [:]).keys.sorted() where !["inner", "outer"].contains(key) {
                report.add(.unsupported, "\(path)/\(side)/\(key)", "此轮盘字段等待支持")
            }
            for ring in ["inner", "outer"] {
                guard let value = wheel[ring] else { continue }
                guard let slots = value.array else {
                    report.add(.unsupported, "\(path)/\(side)/\(ring)", "槽位需要为数组")
                    continue
                }
                var physicalCount = 0
                for (index, slot) in slots.enumerated() {
                    physicalCount += ring == "outer" && slot.array != nil ? 2 : 1
                    if ring == "inner", slot.array != nil {
                        report.add(.unsupported, "\(path)/\(side)/\(ring)/\(index)", "内环控件组等待支持")
                    }
                    scanSlot(
                        slot, path: "\(path)/\(side)/\(ring)/\(index)", assets: assets,
                        report: &report)
                }
                if physicalCount > (ring == "inner" ? 4 : 8) {
                    report.add(.unsupported, "\(path)/\(side)/\(ring)", "控件超过可用物理槽位")
                }
            }
        }
        for (index, slot) in (content["upper"]?["right"]?.array ?? []).enumerated() {
            scanSlot(slot, path: "\(path)/upper/right/\(index)", assets: assets, report: &report)
        }
        for (region, fields) in [
            ("upper", Set(["right"])), ("lower", Set(["leftCenter", "rightCenter", "center"])),
        ] {
            guard let edge = content[region], edge != .null else { continue }
            guard let object = edge.object else {
                report.add(.unsupported, "\(path)/\(region)", "边缘控件区域需要为对象")
                continue
            }
            for key in object.keys where !fields.contains(key) {
                report.add(.unsupported, "\(path)/\(region)/\(key)", "此边缘控件区域等待支持")
            }
            for key in object.keys where fields.contains(key) && key != "center" {
                if object[key]?.array == nil {
                    report.add(.unsupported, "\(path)/\(region)/\(key)", "槽位需要为数组")
                }
            }
        }
        for key in ["leftCenter", "rightCenter"] {
            for (index, slot) in (content["lower"]?[key]?.array ?? []).enumerated() {
                scanSlot(
                    slot, path: "\(path)/lower/\(key)/\(index)", assets: assets, report: &report)
            }
        }
        if let slot = content["lower"]?["center"] {
            scanSlot(slot, path: "\(path)/lower/center", assets: assets, report: &report)
        }
        if let layers = content["layers"]?.object {
            report.behaviors.append("layer")
            for name in layers.keys.sorted() {
                scanContent(
                    layers[name]!, path: "\(path)/layers/\(name)", assets: assets, report: &report)
            }
        }
        if !(content["sensors"]?.array ?? []).isEmpty {
            report.add(.unsupported, "\(path)/sensors", "传感器输入等待宿主接入")
        }
    }

    private static func scanSlot(
        _ slot: TouchJSON, path: String, assets: Set<String>, report: inout TouchCapabilityReport
    ) {
        if slot == .null { return }
        if let items = slot.array {
            report.behaviors.append("cluster")
            for (index, item) in items.enumerated() {
                scanSlot(item, path: "\(path)/\(index)", assets: assets, report: &report)
            }
            if items.count > 4 { report.add(.unsupported, path, "控件组最多支持四项") }
            return
        }
        guard let type = slot["type"]?.string else {
            report.add(.unsupported, path, "控件缺少 type")
            return
        }
        if type == "blank" { return }
        report.controlTypes.append(type)
        guard let kind = TouchControlKind(rawValue: type) else {
            report.add(.unsupported, "\(path)/type", "控件 \(type) 等待支持")
            return
        }
        if kind == .arcadeButtons {
            scanArcadeButtons(slot, path: path, assets: assets, report: &report)
            return
        }
        if (kind == .joystick || kind == .touchpad), slot["axis"] == nil {
            report.add(.unsupported, "\(path)/axis", "此控件需要有效的轴映射")
        }
        if let deadzone = slot["deadzone"], kind == .directionalPad {
            if let value = deadzone.number, value.isFinite, (0...1).contains(value) {
                report.behaviors.append("directionalPadDeadzone")
            } else {
                report.add(.unsupported, "\(path)/deadzone", "方向键死区需要在 0 到 1 之间")
            }
        }
        if let renderAsButton = slot["renderAsButton"], kind == .touchpad,
            renderAsButton.bool == nil
        {
            report.add(.unsupported, "\(path)/renderAsButton", "触摸板按钮外观需要为布尔值")
        }
        for field in ["toggle", "sticky"] where slot[field]?.bool == true {
            report.behaviors.append(field)
        }
        if slot["pullAction"] != nil { report.behaviors.append("pullAction") }
        if kind == .joystick || kind == .throttle {
            report.behaviors.append(slot["relative"]?.bool == true ? "relative" : "absolute")
        }
        if let interaction = slot["interaction"] {
            for key in (interaction.object ?? [:]).keys.sorted() {
                if key != "activationType" || kind != .directionalPad {
                    report.add(.unsupported, "\(path)/interaction/\(key)", "此交互字段等待支持")
                } else if interaction[key]?.string.map({
                    ["exclusive", "allowNeighboring"].contains($0)
                }) != true {
                    report.add(.unsupported, "\(path)/interaction/\(key)", "此方向键激活模式等待支持")
                }
            }
        }
        for field in ["action", "pullAction"] {
            if let action = slot[field] {
                scanAction(action, path: "\(path)/\(field)", report: &report)
            }
        }
        if let axis = slot["axis"] { scanAxis(axis, path: "\(path)/axis", report: &report) }
        for (field, input) in [("axisUp", "axisUp"), ("axisDown", "axisDown")] {
            if let output = slot[field]?.string {
                scanAxis(
                    .object(["input": .string(input), "output": .string(output)]),
                    path: "\(path)/\(field)", report: &report)
            } else if let axis = slot[field] {
                scanAxis(axis, path: "\(path)/\(field)", report: &report)
            }
        }
        scanImages(
            slot["styles"], path: "\(path)/styles", kind: kind, assets: assets, report: &report)
        var fields: Set<String> = [
            "type", "action", "pullAction", "axis", "axisUp", "axisDown", "relative", "expand",
            "sticky", "toggle", "scale", "interaction", "styles", "visible", "enabled",
            "actionThreshold", "priority",
        ]
        if kind == .touchpad { fields.insert("renderAsButton") }
        if kind == .directionalPad { fields.insert("deadzone") }
        for key in (slot.object ?? [:]).keys.sorted() where !fields.contains(key) {
            report.add(.unsupported, "\(path)/\(key)", "此控件字段等待支持")
        }
    }

    private static func scanArcadeButtons(
        _ slot: TouchJSON, path: String, assets: Set<String>, report: inout TouchCapabilityReport
    ) {
        report.behaviors.append("arcadeCombination")
        let required = [
            "lightPunch", "mediumPunch", "heavyPunch", "lightKick", "mediumKick", "heavyKick",
        ]
        let names = required + ["specialPunch", "specialKick"]
        let common: Set<String> = ["type", "scale", "visible", "enabled", "priority"]
        for key in (slot.object ?? [:]).keys.sorted()
        where !common.contains(key) && !names.contains(key) {
            report.add(.unsupported, "\(path)/\(key)", "此格斗控件字段等待支持")
        }
        for name in names {
            guard let button = slot[name], let object = button.object else {
                if required.contains(name) {
                    report.add(.unsupported, "\(path)/\(name)", "格斗控件需要完整的六个按键")
                } else if let value = slot[name], value != .null {
                    report.add(.unsupported, "\(path)/\(name)", "格斗按键需要为对象")
                }
                continue
            }
            if let action = button["action"] {
                scanAction(action, path: "\(path)/\(name)/action", report: &report)
            } else {
                report.add(.unsupported, "\(path)/\(name)/action", "格斗按键需要有效动作")
            }
            for key in object.keys.sorted()
            where !["action", "styles", "visible", "enabled"].contains(key) {
                report.add(.unsupported, "\(path)/\(name)/\(key)", "此格斗按键字段等待支持")
            }
            scanImages(
                button["styles"], path: "\(path)/\(name)/styles", kind: .button,
                assets: assets, report: &report)
        }
    }

    private static func scanAction(
        _ action: TouchJSON, path: String, report: inout TouchCapabilityReport
    ) {
        if let name = action.string {
            if !supportedActions.contains(name) {
                report.add(.unsupported, path, "动作 \(name) 等待支持")
            }
        } else if let items = action.array {
            if items.isEmpty { report.add(.unsupported, path, "动作组需要有效映射") }
            for (index, item) in items.enumerated() {
                scanAction(item, path: "\(path)/\(index)", report: &report)
            }
        } else if action["type"]?.string == "layer", action["target"]?.string != nil {
            return
        } else if action["type"]?.string == "segmented", let items = action["items"]?.array {
            if items.isEmpty { report.add(.unsupported, path, "分段动作需要有效映射") }
            for (index, item) in items.enumerated() {
                scanAction(item, path: "\(path)/items/\(index)", report: &report)
            }
        } else {
            report.add(.unsupported, path, "此动作对象等待支持")
        }
    }

    private static func validateLayerTargets(
        _ value: TouchJSON, path: String, layers: Set<String>, report: inout TouchCapabilityReport
    ) {
        if value["type"]?.string == "layer", let target = value["target"]?.string,
            !layers.contains(target)
        {
            report.add(.unsupported, path, "布局切换目标暂缺")
        }
        for (key, item) in value.object ?? [:] {
            validateLayerTargets(item, path: "\(path)/\(key)", layers: layers, report: &report)
        }
        for (index, item) in (value.array ?? []).enumerated() {
            validateLayerTargets(item, path: "\(path)/\(index)", layers: layers, report: &report)
        }
    }

    private static func scanAxis(
        _ axis: TouchJSON, path: String, report: inout TouchCapabilityReport
    ) {
        if let items = axis.array {
            if items.isEmpty { report.add(.unsupported, path, "轴映射组需要有效映射") }
            for (index, item) in items.enumerated() {
                scanAxis(item, path: "\(path)/\(index)", report: &report)
            }
            return
        }
        guard let input = axis["input"]?.string, let output = axis["output"]?.string,
            supportedInputs.contains(input), supportedOutputs.contains(output)
        else {
            report.add(.unsupported, path, "此轴映射等待支持")
            return
        }
        let vectorOutputs = ["leftJoystick", "rightJoystick"]
        if (input == "axisXY") != vectorOutputs.contains(output) {
            report.add(.unsupported, path, "输入与输出的轴维度需要匹配")
        }
        for key in (axis.object ?? [:]).keys.sorted()
        where !["input", "output", "deadzone", "sensitivity", "responseCurve"].contains(key) {
            report.add(.unsupported, "\(path)/\(key)", "此轴字段等待支持")
        }
        if let curve = axis["responseCurve"] {
            if TouchResponseCurve.parse(curve) != nil {
                report.behaviors.append("responseCurve")
            } else {
                report.add(.unsupported, "\(path)/responseCurve", "响应曲线类型或范围无效")
            }
            for key in (curve.object ?? [:]).keys.sorted() where !["type", "range"].contains(key) {
                report.add(.unsupported, "\(path)/responseCurve/\(key)", "此响应曲线字段等待支持")
            }
        }
        if let sensitivity = axis["sensitivity"],
            sensitivity.number.map({ $0.isFinite && (0...100).contains($0) }) != true
        {
            report.add(.unsupported, "\(path)/sensitivity", "轴灵敏度需要在 0 到 100 之间")
        }
    }

    private static func scanImages(
        _ value: TouchJSON?, path: String, kind: TouchControlKind,
        assets: Set<String>, report: inout TouchCapabilityReport
    ) {
        guard let value else { return }
        if value["type"]?.string == "asset" {
            if let id = value["value"]?.string, assets.contains(id) {
                if !supportsImage(at: path, kind: kind) {
                    report.add(.unsupported, path, "此图片位置等待绘制支持")
                }
            } else {
                report.add(.unsupported, path, "图片资源等待布局包加载")
            }
        }
        if value["type"]?.string == "icon", let name = value["value"]?.string,
            !supportedIcons.contains(name)
        {
            report.add(.warning, path, "图标 \(name) 使用动作符号回退")
        }
        for (key, item) in value.object ?? [:] {
            scanImages(item, path: "\(path)/\(key)", kind: kind, assets: assets, report: &report)
        }
        for (index, item) in (value.array ?? []).enumerated() {
            scanImages(item, path: "\(path)/\(index)", kind: kind, assets: assets, report: &report)
        }
    }

    private static func supportsImage(at path: String, kind: TouchControlKind) -> Bool {
        let components = path.split(separator: "/").map(String.init)
        guard let styles = components.lastIndex(of: "styles"), components.count > styles + 2 else {
            return false
        }
        let states: Set<String>
        switch kind {
        case .button: states = ["default", "idle", "activated", "disabled", "toggled", "pulled"]
        case .joystick, .touchpad: states = ["default", "idle", "activated", "disabled", "moving"]
        case .throttle:
            states = ["default", "idle", "idleUp", "activatedUp", "activatedDown", "disabled"]
        case .directionalPad: states = ["default", "idle", "activated", "disabled"]
        case .arcadeButtons: return false
        }
        guard states.contains(components[styles + 1]) else { return false }
        let location = Array(components.dropFirst(styles + 2))
        if location == ["background"] || location == ["faceImage"] { return true }
        if (kind == .joystick || kind == .throttle),
            location == ["knob", "background"] || location == ["knob", "faceImage"]
        {
            return true
        }
        if kind == .button, location.first == "pullIndicator" {
            if location == ["pullIndicator", "faceImage"] { return true }
            if location.count == 3, let index = Int(location[1]), (0..<4).contains(index) {
                return location[2] == "faceImage"
            }
        }
        return false
    }
}
