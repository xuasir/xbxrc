import Foundation

enum TouchLayoutResolver {
    static func resolve(_ layout: TouchLayout, activeLayers: [String]) -> TouchJSON {
        var content = layout.content
        let layers = layout.content["layers"]?.object ?? [:]
        for name in activeLayers {
            guard let patch = layers[name] else { continue }
            content = mergeContent(content, patch)
        }
        return content
    }

    /// 对齐官方 sh()：cluster 与单槽切换会调整后一槽，保持两个物理槽的占位。
    static func mergeSlots(_ base: [TouchJSON], _ patch: [TouchJSON]) -> [TouchJSON] {
        var result = base
        var index = 0
        while index < result.count {
            if index < patch.count {
                let previous = result[index]
                let next = patch[index]
                if next == .null {
                    index += 1
                    continue
                }
                if let oldItems = previous.array, let newItems = next.array {
                    result[index] =
                        oldItems.count == newItems.count
                        ? .array(mergeSlots(oldItems, newItems)) : next
                } else if previous.array != nil || next.array != nil {
                    result[index] = next
                    if next.array != nil {
                        if index + 1 < result.count {
                            if result[index + 1].array != nil {
                                result[index + 1] = .null
                            } else {
                                result.remove(at: index + 1)
                            }
                        }
                    } else if index + 1 < result.count {
                        result.insert(.null, at: index + 1)
                    }
                } else if next != .null {
                    result[index] = next
                }
            }
            index += 1
        }
        if patch.count > result.count { result.append(contentsOf: patch.dropFirst(result.count)) }
        return result
    }

    private static func mergeWheel(_ base: TouchJSON?, _ patch: TouchJSON) -> TouchJSON {
        guard var result = base?.object else { return patch }
        for ring in ["inner", "outer"] {
            guard let slots = patch[ring]?.array else { continue }
            result[ring] = .array(mergeSlots(base?[ring]?.array ?? [], slots))
        }
        return .object(result)
    }

    private static func mergeContent(_ base: TouchJSON, _ patch: TouchJSON) -> TouchJSON {
        var result = base.object ?? [:]
        for wheel in ["left", "right", "center"] {
            if let value = patch[wheel], value != .null {
                result[wheel] = mergeWheel(base[wheel], value)
            }
        }
        for edge in ["upper", "lower"] {
            guard let value = patch[edge], value != .null else { continue }
            var region = base[edge]?.object ?? [:]
            for key in (value.object ?? [:]).keys.sorted() {
                let next = value[key]!
                if let slots = next.array {
                    region[key] = .array(mergeSlots(base[edge]?[key]?.array ?? [], slots))
                } else if next != .null {
                    region[key] = next
                }
            }
            result[edge] = .object(region)
        }
        return .object(result)
    }
}
