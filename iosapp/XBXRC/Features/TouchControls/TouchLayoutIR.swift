import Foundation

/// 公共 Rust 编译协议结构与能力；Swift 消费类型化控件和原生绘制描述。
struct TouchCompiledControl: Codable, Equatable, Sendable {
  let id: String
  let kind: TouchControlKind
  let action: TouchAction?
  let pullAction: TouchAction?
  let axis: [TouchAxisMapping]
  let axisUp: [TouchAxisMapping]
  let axisDown: [TouchAxisMapping]
  let activationType: String
  let styles: TouchJSON
  let children: [String: TouchCompiledControl]
}

struct TouchLayoutIR: Decodable, Sendable {
  struct Issue: Decodable, Sendable {
    let severity: TouchCapabilityIssue.Severity
    let path: String
    let message: String
    let capability: String
  }
  let irVersion: Int
  let protocolRevision: String
  let id: String
  let name: String
  let author: String?
  let format: String
  let schema: String
  let content: TouchJSON
  let controls: [String: TouchCompiledControl]
  let colorPalette: TouchJSON
  let availableAssets: [String]
  let runtimeDocument: String
  let requiredCapabilities: [String]
  let controlTypes: [String]
  let behaviors: [String]
  let issues: [Issue]
  let isRunnable: Bool
  let isFullyCompatible: Bool
  let usesStandardFallback: Bool

  var layout: TouchLayout {
    var report = TouchCapabilityReport(format: format, schema: schema)
    report.controlTypes = controlTypes
    report.behaviors = behaviors
    report.runnable = isRunnable
    report.fullyCompatible = isFullyCompatible
    report.requiredCapabilities = requiredCapabilities
    report.usesStandardFallback = usesStandardFallback
    report.issues = issues.map {
      .init(severity: $0.severity, path: $0.path, message: $0.message, capability: $0.capability)
    }
    return TouchLayout(
      id: id, name: name, author: author, content: content, report: report,
      runtimeDocument: runtimeDocument, colorPalette: colorPalette,
      availableAssets: Set(availableAssets), compiledControls: controls,
      protocolRevision: protocolRevision)
  }
}

enum TouchProtocolCatalog {
  struct Information: Decodable, Sendable {
    let irVersion: Int
    let revision: String
    let iconNames: [String]
    let systemColors: [String: String]
  }
  static let information: Information = {
    let json = touchLayoutProtocolInformationJson()
    return try! JSONDecoder().decode(Information.self, from: Data(json.utf8))
  }()
}

extension TouchAction: Codable {
  private enum Keys: String, CodingKey { case kind, name, target, items, action, interval }
  init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: Keys.self)
    switch try container.decode(String.self, forKey: .kind) {
    case "gamepad": self = .gamepad(try container.decode(String.self, forKey: .name))
    case "layer": self = .layer(try container.decode(String.self, forKey: .target))
    case "group": self = .group(try container.decode([Self].self, forKey: .items))
    case "segmented": self = .segmented(try container.decode([Self].self, forKey: .items))
    case "turbo":
      self = .turbo(
        try container.decode(Self.self, forKey: .action),
        try container.decode(Double.self, forKey: .interval))
    default:
      throw DecodingError.dataCorruptedError(
        forKey: .kind, in: container, debugDescription: "Unknown compiled action")
    }
  }
  func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: Keys.self)
    switch self {
    case .gamepad(let name):
      try container.encode("gamepad", forKey: .kind)
      try container.encode(name, forKey: .name)
    case .layer(let target):
      try container.encode("layer", forKey: .kind)
      try container.encode(target, forKey: .target)
    case .group(let items):
      try container.encode("group", forKey: .kind)
      try container.encode(items, forKey: .items)
    case .segmented(let items):
      try container.encode("segmented", forKey: .kind)
      try container.encode(items, forKey: .items)
    case .turbo(let action, let interval):
      try container.encode("turbo", forKey: .kind)
      try container.encode(action, forKey: .action)
      try container.encode(interval, forKey: .interval)
    }
  }
}
