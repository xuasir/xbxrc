import Foundation

struct TouchLayoutChoice: Identifiable, Equatable, Sendable {
    let id: String
    let title: String
    let titleID: String?
    let layout: TouchLayout
}

enum TouchLayoutCatalog {
    static func standard(bundle: Bundle = .main) throws -> TouchLayout {
        guard let layout = try read("generic", bundle: bundle).first else {
            throw TouchLayoutError.invalid("通用 Xbox 布局为空")
        }
        return layout
    }

    static func read(_ resource: String, bundle: Bundle = .main) throws -> [TouchLayout] {
        guard
            let url = bundle.url(
                forResource: resource, withExtension: "json", subdirectory: "TouchLayouts")
                ?? bundle.url(forResource: resource, withExtension: "json")
        else { throw TouchLayoutError.invalid("资源 \(resource) 缺失") }
        return try TouchLayoutNormalizer.decode(Data(contentsOf: url), defaultID: resource)
    }

    static func builtIn(bundle: Bundle = .main) throws -> [TouchLayoutChoice] {
        let generic = try read("generic", bundle: bundle)
        let forza = try read("forza-layouts", bundle: bundle)
        let official = try read("official-layout", bundle: bundle)
        return generic.map {
            .init(id: "generic:\($0.id)", title: "通用 Xbox", titleID: nil, layout: $0)
        }
            + forza.map {
                .init(
                    id: "forza:\($0.id)",
                    title: $0.id.contains("throttle") ? "地平线 5 · 踏板" : "地平线 5 · 按键",
                    titleID: "2030093255", layout: $0)
            }
            + official.map {
                .init(id: "official:\($0.id)", title: "官方格式示例", titleID: nil, layout: $0)
            }
    }
}
