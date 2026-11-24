import CoreGraphics
import Foundation

struct TouchSurfaceInsets: Equatable, Sendable {
    var top: CGFloat = 0
    var left: CGFloat = 0
    var bottom: CGFloat = 0
    var right: CGFloat = 0
}

struct TouchControlSocket: Equatable, Sendable {
    let control: TouchControl
    let center: CGPoint
    let diameter: CGFloat
    let unitSize: CGFloat
    let regionOrder: Int
    var id: String { control.id }
    var collisionRadius: CGFloat { max(22, diameter / 2) }
    var frame: CGRect {
        .init(
            x: center.x - diameter / 2, y: center.y - diameter / 2,
            width: diameter, height: diameter)
    }

    func contains(_ point: CGPoint) -> Bool {
        hypot(point.x - center.x, point.y - center.y) <= collisionRadius
    }
}

enum TouchLayoutGeometry {
    static func availableRect(size: CGSize, insets: TouchSurfaceInsets = .init()) -> CGRect {
        CGRect(
            x: insets.left + 12, y: insets.top + 12,
            width: max(1, size.width - insets.left - insets.right - 24),
            height: max(1, size.height - insets.top - insets.bottom - 24))
    }

    static func configured(
        _ sockets: [TouchControlSocket], size: CGSize,
        insets: TouchSurfaceInsets, configuration: TouchLayoutConfiguration
    ) -> [TouchControlSocket] {
        let available = availableRect(size: size, insets: insets)
        let scale = configuration.scale.isFinite ? min(1.5, max(0.75, configuration.scale)) : 1
        return sockets.map { socket in
            let diameter = min(socket.diameter * scale, min(available.width, available.height))
            let radius = max(22, diameter / 2)
            var center = socket.center
            if let point = configuration.positions[socket.id], point.x.isFinite, point.y.isFinite {
                center = CGPoint(
                    x: available.minX + point.x * available.width,
                    y: available.minY + point.y * available.height)
            }
            func clamp(_ value: CGFloat, _ lower: CGFloat, _ upper: CGFloat) -> CGFloat {
                lower <= upper ? min(upper, max(lower, value)) : (lower + upper) / 2
            }
            center.x = clamp(center.x, available.minX + radius, available.maxX - radius)
            center.y = clamp(center.y, available.minY + radius, available.maxY - radius)
            return .init(
                control: socket.control, center: center, diameter: diameter,
                unitSize: socket.unitSize * diameter / socket.diameter,
                regionOrder: socket.regionOrder)
        }
    }

    static func normalizedPosition(
        _ point: CGPoint, size: CGSize, insets: TouchSurfaceInsets = .init()
    ) -> CGPoint {
        let available = availableRect(size: size, insets: insets)
        return CGPoint(
            x: min(1, max(0, (point.x - available.minX) / available.width)),
            y: min(1, max(0, (point.y - available.minY) / available.height)))
    }

    static func sockets(
        content: TouchJSON, size: CGSize, insets: TouchSurfaceInsets = .init()
    ) -> [TouchControlSocket] {
        guard size.width > 0, size.height > 0 else { return [] }
        let available = CGRect(
            x: insets.left + 12, y: insets.top + 12,
            width: max(1, size.width - insets.left - insets.right - 24),
            height: max(1, size.height - insets.top - insets.bottom - 24))
        let unit = max(28, min(52, min(available.width / 11.5, available.height / 6.6)))
        let outerRadius = unit * 2.5
        let margin = unit * 3.0
        let centerY = max(available.midY, available.maxY - unit * 2.9)
        var result: [TouchControlSocket] = []
        for (order, side) in ["left", "center", "right"].enumerated() {
            guard let wheel = content[side] else { continue }
            let centerX: CGFloat
            switch side {
            case "left": centerX = available.minX + margin
            case "right": centerX = available.maxX - margin
            default: centerX = available.midX
            }
            let center = CGPoint(x: centerX, y: centerY)
            let inner = wheel["inner"]?.array ?? []
            for (index, item) in inner.enumerated().prefix(4) {
                let angle = CGFloat(index) * 2 * .pi / CGFloat(max(1, inner.count))
                let offset = inner.count == 1 ? 0 : unit * 1.1
                let point = CGPoint(
                    x: center.x + cos(angle) * offset,
                    y: center.y + sin(angle) * offset)
                append(
                    item, id: "\(side)/inner/\(index)", center: point,
                    diameter: inner.count == 1 ? unit * 2.3 : unit * 1.2,
                    unit: unit, order: order, to: &result)
            }
            var physicalSlot = 0
            for item in wheel["outer"]?.array ?? [] {
                guard physicalSlot < 8 else { break }
                let angle = -.pi / 2 + CGFloat(physicalSlot) * .pi / 4
                let id = "\(side)/outer/\(physicalSlot)"
                if let cluster = item.array {
                    let middle = angle + .pi / 8
                    let root = CGPoint(
                        x: center.x + cos(middle) * outerRadius,
                        y: center.y + sin(middle) * outerRadius)
                    for (index, child) in cluster.enumerated().prefix(4) {
                        let point: CGPoint
                        if cluster.count == 1 {
                            point = root
                        } else if cluster.count == 2 {
                            let shift = (CGFloat(index) - 0.5) * unit * 1.2
                            point = .init(
                                x: root.x + cos(middle + .pi / 2) * shift,
                                y: root.y + sin(middle + .pi / 2) * shift)
                        } else {
                            let childAngle =
                                middle + .pi + CGFloat(index) * 2 * .pi / CGFloat(cluster.count)
                            point = .init(
                                x: root.x + cos(childAngle) * unit * 0.78,
                                y: root.y + sin(childAngle) * unit * 0.78)
                        }
                        append(
                            child, id: "\(id)/\(index)", center: point,
                            diameter: unit * 1.12, unit: unit, order: order, to: &result)
                    }
                    physicalSlot += 2
                } else {
                    let point = CGPoint(
                        x: center.x + cos(angle) * outerRadius,
                        y: center.y + sin(angle) * outerRadius)
                    append(
                        item, id: id, center: point, diameter: unit * 1.12,
                        unit: unit, order: order, to: &result)
                    physicalSlot += 1
                }
            }
        }
        for (index, item) in (content["upper"]?["right"]?.array ?? []).enumerated() {
            append(
                item, id: "upper/right/\(index)",
                center: .init(
                    x: available.maxX - unit * (CGFloat(index) + 0.6) * 1.25,
                    y: available.minY + unit * 0.6),
                diameter: unit, unit: unit, order: 3, to: &result)
        }
        for side in ["leftCenter", "rightCenter"] {
            let items = content["lower"]?[side]?.array ?? []
            let anchorX =
                side == "leftCenter"
                ? available.midX - unit * 1.8 : available.midX + unit * 1.8
            for (index, item) in items.enumerated() {
                append(
                    item, id: "lower/\(side)/\(index)",
                    center: .init(
                        x: anchorX + (CGFloat(index) - CGFloat(items.count - 1) / 2) * unit * 1.25,
                        y: available.maxY - unit * 0.6),
                    diameter: unit, unit: unit, order: 4, to: &result)
            }
        }
        if let item = content["lower"]?["center"] {
            append(
                item, id: "lower/center",
                center: .init(x: available.midX, y: available.maxY - unit * 0.6),
                diameter: unit, unit: unit, order: 4, to: &result)
        }
        return result
    }

    static func hitTest(_ point: CGPoint, sockets: [TouchControlSocket]) -> TouchControlSocket? {
        sockets.filter { $0.control.visible && $0.contains(point) }.min {
            if $0.regionOrder != $1.regionOrder { return $0.regionOrder < $1.regionOrder }
            if $0.control.priority != $1.control.priority {
                return $0.control.priority > $1.control.priority
            }
            let first = hypot(point.x - $0.center.x, point.y - $0.center.y) / $0.collisionRadius
            let second = hypot(point.x - $1.center.x, point.y - $1.center.y) / $1.collisionRadius
            return first == second ? $0.id < $1.id : first < second
        }
    }

    private static func append(
        _ item: TouchJSON, id: String, center: CGPoint, diameter: CGFloat,
        unit: CGFloat, order: Int, to result: inout [TouchControlSocket]
    ) {
        guard let type = item["type"]?.string, let kind = TouchControlKind(rawValue: type),
            kind != .touchpad
        else { return }
        let control = TouchControl(id: id, kind: kind, properties: item)
        guard control.visible else { return }
        result.append(
            .init(
                control: control, center: center,
                diameter: max(20, diameter * CGFloat(control.scale)), unitSize: unit,
                regionOrder: order))
    }
}
