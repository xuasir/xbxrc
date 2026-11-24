import CoreGraphics
import Foundation

enum TouchLayoutOrientation: String, Codable, CaseIterable, Identifiable, Sendable {
    case landscape, portrait
    var id: String { rawValue }
    var title: String { self == .landscape ? "横屏" : "竖屏" }
    var previewSize: CGSize {
        self == .landscape ? CGSize(width: 844, height: 390) : CGSize(width: 390, height: 844)
    }
    init(size: CGSize) { self = size.height > size.width ? .portrait : .landscape }
}

struct TouchLayoutAppearance: Codable, Equatable, Sendable {
    var opacity = 0.65
    var scale = 1.0
    static let opacityRange = 0.2...1.0
    static let scaleRange = 0.75...1.5

    func normalized() -> Self {
        .init(
            opacity: opacity.isFinite ? min(1, max(0.2, opacity)) : 0.65,
            scale: scale.isFinite ? min(1.5, max(0.75, scale)) : 1)
    }
}

/// 模板只记录位置和外观；Xbox 动作与图标继续来自随包标准布局。
struct TouchLayoutTemplate: Codable, Equatable, Identifiable, Sendable {
    static let maximumCount = 20
    static let standardID = "classic"
    var version = 1
    var id = UUID().uuidString
    var name = ""
    var landscape: [String: TouchPoint] = [:]
    var portrait: [String: TouchPoint] = [:]
    var appearance = TouchLayoutAppearance()

    func positions(for orientation: TouchLayoutOrientation) -> [String: TouchPoint] {
        orientation == .landscape ? landscape : portrait
    }

    mutating func setPosition(
        _ position: CGPoint, for id: String, orientation: TouchLayoutOrientation
    ) {
        if orientation == .landscape {
            landscape[id] = TouchPoint(position)
        } else {
            portrait[id] = TouchPoint(position)
        }
    }

    mutating func resetPositions(for orientation: TouchLayoutOrientation) {
        if orientation == .landscape { landscape = [:] } else { portrait = [:] }
    }

    func normalized(validControlIDs: Set<String>) -> Self? {
        guard version == 1, let uuid = UUID(uuidString: id) else { return nil }
        var value = self
        value.id = uuid.uuidString
        value.name = String(name.trimmingCharacters(in: .whitespacesAndNewlines).prefix(40))
        value.appearance = appearance.normalized()
        func positions(_ source: [String: TouchPoint]) -> [String: TouchPoint] {
            source.filter {
                validControlIDs.contains($0.key) && $0.value.x.isFinite && $0.value.y.isFinite
            }
            .mapValues { TouchPoint(CGPoint(x: min(1, max(0, $0.x)), y: min(1, max(0, $0.y)))) }
        }
        value.landscape = positions(landscape)
        value.portrait = positions(portrait)
        return value
    }

    static func normalized(_ templates: [Self]) -> [Self] {
        var ids: Set<String> = []
        return templates.compactMap { template in
            guard let value = template.normalized(validControlIDs: standardControlIDs),
                ids.insert(value.id).inserted
            else { return nil }
            return value
        }.prefix(maximumCount).map { $0 }
    }

    // 随包资源只解析一次，设置归一化与编辑器共享当前标准控件身份。
    static let standardControlIDs: Set<String> = {
        guard let layout = try? TouchLayoutCatalog.standard() else { return [] }
        return Set(
            TouchLayoutGeometry.sockets(
                content: layout.content, size: CGSize(width: 844, height: 390)
            ).map(\.id))
    }()
}

/// 某个模板损坏时仍保留同一偏好快照中的其他有效模板。
struct TouchLayoutTemplateList: Decodable {
    let templates: [TouchLayoutTemplate]
    init(from decoder: any Decoder) throws {
        var container = try decoder.unkeyedContainer()
        var result: [TouchLayoutTemplate] = []
        while !container.isAtEnd {
            let item = try container.superDecoder()
            if let template = try? TouchLayoutTemplate(from: item) { result.append(template) }
        }
        templates = result
    }
}

struct TouchLayoutConfiguration: Equatable, Sendable {
    var identity = TouchLayoutTemplate.standardID
    var positions: [String: TouchPoint] = [:]
    var scale = 1.0
}

extension MobilePreferences {
    var selectedTouchTemplate: TouchLayoutTemplate? {
        touchLayoutTemplates.first { $0.id == touchLayoutID }
    }
    var selectedTouchLayoutName: String { selectedTouchTemplate?.name ?? "通用 Xbox" }
    var selectedTouchAppearance: TouchLayoutAppearance {
        selectedTouchTemplate?.appearance ?? .init(opacity: touchOpacity, scale: touchScale)
    }
    func touchConfiguration(for orientation: TouchLayoutOrientation) -> TouchLayoutConfiguration {
        .init(
            identity: touchLayoutID,
            positions: selectedTouchTemplate?.positions(for: orientation) ?? [:],
            scale: selectedTouchAppearance.scale)
    }

    func newTouchTemplateName() -> String {
        let names = Set(touchLayoutTemplates.map(\.name))
        var number = 1
        var name: String
        repeat {
            name = String.localizedStringWithFormat(
                NSLocalizedString("自定义布局 %lld", comment: ""), number)
            number += 1
        } while names.contains(name)
        return name
    }

    @discardableResult
    mutating func saveTouchTemplate(_ draft: TouchLayoutTemplate, asNew: Bool = false) -> Bool {
        var template = draft
        if asNew { template.id = UUID().uuidString }
        guard
            var normalized = template.normalized(
                validControlIDs: TouchLayoutTemplate.standardControlIDs)
        else { return false }
        if normalized.name.isEmpty { normalized.name = newTouchTemplateName() }
        if let index = touchLayoutTemplates.firstIndex(where: { $0.id == normalized.id }) {
            touchLayoutTemplates[index] = normalized
        } else {
            guard touchLayoutTemplates.count < TouchLayoutTemplate.maximumCount else {
                return false
            }
            touchLayoutTemplates.append(normalized)
        }
        touchLayoutID = normalized.id
        return true
    }

    mutating func deleteTouchTemplate(_ id: String) {
        touchLayoutTemplates.removeAll { $0.id == id }
        if touchLayoutID == id { touchLayoutID = TouchLayoutTemplate.standardID }
        if recentCustomTouchLayoutID == TouchLayoutSelection.template(id).persistentID {
            recentCustomTouchLayoutID = nil
        }
    }

    mutating func saveTouchAppearance(_ appearance: TouchLayoutAppearance, for id: String) {
        let appearance = appearance.normalized()
        if id == TouchLayoutTemplate.standardID {
            touchOpacity = appearance.opacity
            touchScale = appearance.scale
        } else if let index = touchLayoutTemplates.firstIndex(where: { $0.id == id }) {
            touchLayoutTemplates[index].appearance = appearance
        }
    }
}
