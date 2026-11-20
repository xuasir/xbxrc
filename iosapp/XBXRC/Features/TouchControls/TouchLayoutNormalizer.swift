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

    static func decode(_ data: Data, defaultID: String = "layout") throws -> [TouchLayout] {
        guard data.count <= 2_000_000 else { throw TouchLayoutError.invalid("文件超过 2 MB") }
        let root = try JSONDecoder().decode(TouchJSON.self, from: data)
        guard root.object != nil else { throw TouchLayoutError.invalid("根节点需要为对象") }
        if let layouts = root["layouts"]?.object {
            guard root["schema_version"]?.number == 1 else {
                throw TouchLayoutError.invalid("社区包装版本需要为 1")
            }
            return try layouts.keys.sorted().map {
                try normalize(layouts[$0]!, id: $0, format: "community")
            }
        }
        return [try normalize(root, id: defaultID, format: "official")]
    }

    private static func normalize(_ document: TouchJSON, id: String, format: String) throws
        -> TouchLayout
    {
        guard let content = document["content"], content.object != nil else {
            throw TouchLayoutError.invalid("\(id) 缺少 content")
        }
        var report = TouchCapabilityReport(
            format: format, schema: document["$schema"]?.string ?? "runtime-v1")
        if let schema = document["$schema"]?.string,
            !schema.contains("/layout/latest/") && !schema.contains("/layout/3.5/")
        {
            report.add(.unsupported, "/$schema", "此布局版本等待转换器支持")
        }
        let value = normalizeNode(content, path: "/content", depth: 0, report: &report)
        scanContent(value, path: "/content", report: &report)
        report.controlTypes = Array(Set(report.controlTypes)).sorted()
        report.behaviors = Array(Set(report.behaviors)).sorted()
        return TouchLayout(
            id: id, name: document["name"]?.string ?? document["displayName"]?.string ?? id,
            author: document["author"]?.string, content: value, report: report)
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
        _ content: TouchJSON, path: String, report: inout TouchCapabilityReport
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
                    scanSlot(slot, path: "\(path)/\(side)/\(ring)/\(index)", report: &report)
                }
                if physicalCount > (ring == "inner" ? 4 : 8) {
                    report.add(.unsupported, "\(path)/\(side)/\(ring)", "控件超过可用物理槽位")
                }
            }
        }
        for (index, slot) in (content["upper"]?["right"]?.array ?? []).enumerated() {
            scanSlot(slot, path: "\(path)/upper/right/\(index)", report: &report)
        }
        for key in ["leftCenter", "rightCenter"] {
            for (index, slot) in (content["lower"]?[key]?.array ?? []).enumerated() {
                scanSlot(slot, path: "\(path)/lower/\(key)/\(index)", report: &report)
            }
        }
        if let slot = content["lower"]?["center"] {
            scanSlot(slot, path: "\(path)/lower/center", report: &report)
        }
        if let layers = content["layers"]?.object {
            report.behaviors.append("layer")
            for name in layers.keys.sorted() {
                scanContent(layers[name]!, path: "\(path)/layers/\(name)", report: &report)
            }
        }
        if !(content["sensors"]?.array ?? []).isEmpty {
            report.add(.unsupported, "\(path)/sensors", "传感器输入等待宿主接入")
        }
    }

    private static func scanSlot(
        _ slot: TouchJSON, path: String, report: inout TouchCapabilityReport
    ) {
        if slot == .null { return }
        if let items = slot.array {
            report.behaviors.append("cluster")
            for (index, item) in items.enumerated() {
                scanSlot(item, path: "\(path)/\(index)", report: &report)
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
        if kind == .touchpad {
            report.add(.unsupported, path, "触摸板输入等待速度/鼠标映射支持")
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
                } else if let value = interaction[key]?.string,
                    !["exclusive", "allowNeighboring"].contains(value)
                {
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
        scanImages(slot["styles"], path: "\(path)/styles", report: &report)
        let fields: Set<String> = [
            "type", "action", "pullAction", "axis", "axisUp", "axisDown", "relative", "expand",
            "sticky", "toggle", "scale", "interaction", "styles", "visible", "enabled",
            "actionThreshold", "priority",
        ]
        for key in (slot.object ?? [:]).keys.sorted() where !fields.contains(key) {
            report.add(.unsupported, "\(path)/\(key)", "此控件字段等待支持")
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
            for (index, item) in items.enumerated() {
                scanAction(item, path: "\(path)/\(index)", report: &report)
            }
        } else if action["type"]?.string == "layer", action["target"]?.string != nil {
            return
        } else if action["type"]?.string == "segmented", let items = action["items"]?.array {
            for (index, item) in items.enumerated() {
                scanAction(item, path: "\(path)/items/\(index)", report: &report)
            }
        } else {
            report.add(.unsupported, path, "此动作对象等待支持")
        }
    }

    private static func scanAxis(
        _ axis: TouchJSON, path: String, report: inout TouchCapabilityReport
    ) {
        if let items = axis.array {
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
        if axis["responseCurve"] != nil {
            report.add(.unsupported, "\(path)/responseCurve", "响应曲线等待有限值回归")
        }
    }

    private static func scanImages(
        _ value: TouchJSON?, path: String, report: inout TouchCapabilityReport
    ) {
        guard let value else { return }
        if value["type"]?.string == "asset" {
            report.add(.unsupported, path, "图片资源等待布局包加载")
        }
        if value["type"]?.string == "icon", let name = value["value"]?.string,
            !supportedIcons.contains(name)
        {
            report.add(.warning, path, "图标 \(name) 使用动作符号回退")
        }
        for (key, item) in value.object ?? [:] {
            scanImages(item, path: "\(path)/\(key)", report: &report)
        }
    }
}
