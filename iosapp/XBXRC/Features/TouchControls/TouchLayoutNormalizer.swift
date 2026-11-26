import Foundation

/// 来源适配后交由公共 Rust 协议编译器生成同一份布局 IR。
enum TouchLayoutNormalizer {
  static func decode(
    _ data: Data, defaultID: String = "layout", availableAssets: Set<String> = [],
    stateJSON: String? = nil, patchJSON: String? = nil, format: String? = nil
  ) throws -> [TouchLayout] {
    guard data.count <= 8 * 1024 * 1024 else { throw TouchLayoutError.invalid("布局超过编译预算") }
    let root = try JSONDecoder().decode(TouchJSON.self, from: data)
    guard root.object != nil else { throw TouchLayoutError.invalid("根节点需要为对象") }
    if let layouts = root["layouts"]?.object {
      return try layouts.keys.sorted().map { id in
        var layout = try compile(
          layouts[id]!, id: id, format: "community", assets: availableAssets,
          stateJSON: stateJSON, patchJSON: patchJSON)
        if root["schema_version"]?.number != 1 {
          layout.report.add(.warning, "/schema_version", "社区包装按已知布局字段运行", capability: "version")
        }
        return layout
      }
    }
    return [
      try compile(
        root, id: defaultID, format: format ?? "official", assets: availableAssets,
        stateJSON: stateJSON, patchJSON: patchJSON)
    ]
  }

  /// 本机快照由 Rust 校验来源及修订；资源变化或修订升级时重新编译原文。
  static func preparedSnapshot(_ data: Data, assets: Set<String>) throws -> TouchLayout? {
    guard data.count <= 8 * 1024 * 1024 else { throw TouchLayoutError.invalid("编译快照超过预算") }
    guard let ir = try? JSONDecoder().decode(TouchLayoutIR.self, from: data),
      ir.irVersion == TouchProtocolCatalog.information.irVersion,
      ir.protocolRevision == TouchProtocolCatalog.information.revision,
      Set(ir.availableAssets) == assets
    else { return nil }
    return ir.layout
  }

  private static func compile(
    _ source: TouchJSON, id: String, format: String, assets: Set<String>,
    stateJSON: String?, patchJSON: String?
  ) throws -> TouchLayout {
    let result = try compileTouchLayoutJson(
      documentJson: source.encodedJSON(), layoutId: id, format: format,
      availableAssets: assets.sorted(), stateJson: stateJSON, patchJson: patchJSON)
    let ir = try JSONDecoder().decode(TouchLayoutIR.self, from: Data(result.utf8))
    guard ir.irVersion == TouchProtocolCatalog.information.irVersion else {
      throw TouchLayoutError.invalid("编译布局版本需要更新")
    }
    return ir.layout
  }
}
