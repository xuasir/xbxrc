import CoreGraphics
import Foundation
import ImageIO

extension XboxOfficialTouchBundleChoice: Identifiable {}
extension XboxTouchLayoutChoice: Identifiable {}
extension XboxSavedTouchLayout: Identifiable {}

enum GameTouchLayoutSource: String, Sendable {
  case official
  case community

  var title: String { self == .official ? "官方布局" : "社区布局" }
  var shortTitle: String { self == .official ? "官方" : "社区" }

  static func identify(_ bundleID: String) -> Self {
    bundleID.hasPrefix("community:better-xcloud") ? .community : .official
  }
}

extension XboxOfficialTouchBundleChoice {
  var source: GameTouchLayoutSource { .identify(id) }
  var savedBundleID: String { source == .community ? "community:better-xcloud" : id }
}

extension XboxSavedTouchLayout {
  var source: GameTouchLayoutSource { .identify(bundleId) }
}

struct TouchCatalogSourceResult: Sendable {
  let source: GameTouchLayoutSource
  let bundles: [XboxOfficialTouchBundleChoice]
  let failure: SectionRequestFailure?
}

protocol OfficialTouchBundleProviding: Sendable {
  var version: String { get }
  var choices: [XboxTouchLayoutChoice] { get }
  func preview(layoutID: String) async throws -> XboxTouchLayoutSnapshot
}

struct OfficialTouchCatalog: Sendable {
  let titleID: String
  let bundles: [XboxOfficialTouchBundleChoice]
  var sources: [TouchCatalogSourceResult] = []
}

struct StreamingGameTouchLayoutChoice: Identifiable, Sendable {
  let bundleID: String
  let version: String
  let choice: XboxTouchLayoutChoice
  var id: String { bundleID + ":" + choice.id }
  var source: GameTouchLayoutSource { .identify(bundleID) }
  var selection: TouchLayoutSelection {
    .gameLayout(bundleID: bundleID, layoutID: choice.id)
  }
}

/// 后台解码后只读共享，绘制阶段直接交付 CGImage。
struct TouchLayoutAssets: @unchecked Sendable {
  let images: [String: CGImage]
  static let empty = TouchLayoutAssets(images: [:])
}

struct PreparedTouchLayout: Identifiable, Sendable {
  let layout: TouchLayout
  let assets: TouchLayoutAssets
  var id: String { layout.id }

  static func prepare(_ snapshot: XboxTouchLayoutSnapshot) async throws -> Self {
    let task = Task.detached(priority: .userInitiated) {
      try Task.checkCancellation()
      guard snapshot.assets.count <= 256 else {
        throw TouchLayoutError.invalid("图片资源数量超出限制")
      }
      var images: [String: CGImage] = [:]
      var resourceWarnings: [String] = []
      var decodedBytes = 0
      for asset in snapshot.assets {
        try Task.checkCancellation()
        guard images[asset.id] == nil,
          let source = CGImageSourceCreateWithData(Data(asset.data) as CFData, nil),
          CGImageSourceGetCount(source) == 1,
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil)
            as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int,
          width > 0, height > 0, width <= 4096, height <= 4096
        else {
          resourceWarnings.append("此图片资源使用默认图形")
          continue
        }
        let nextBytes = decodedBytes + width * height * 4
        guard nextBytes <= 64 * 1024 * 1024,
          let image = CGImageSourceCreateImageAtIndex(
            source, 0, [kCGImageSourceShouldCacheImmediately: true] as CFDictionary)
        else {
          resourceWarnings.append("图片解码达到预算，使用默认图形")
          continue
        }
        decodedBytes = nextBytes
        images[asset.id] = image
      }
      guard
        let decoded = try TouchLayoutNormalizer.preparedSnapshot(
          Data(snapshot.documentJson.utf8), assets: Set(images.keys))
          ?? TouchLayoutNormalizer.decode(
            Data(snapshot.documentJson.utf8), defaultID: snapshot.id,
            availableAssets: Set(images.keys)
          ).first
      else { throw TouchLayoutError.invalid("布局内容为空") }
      var report = decoded.report
      for issue in snapshot.issues + resourceWarnings {
        report.add(.warning, "/bundle", issue, capability: "resource.bundle")
      }
      let layout = TouchLayout(
        id: snapshot.id, name: snapshot.name, author: decoded.author,
        content: decoded.content, report: report,
        runtimeDocument: decoded.runtimeDocument, colorPalette: decoded.colorPalette,
        availableAssets: decoded.availableAssets, compiledControls: decoded.compiledControls,
        protocolRevision: decoded.protocolRevision)
      return Self(layout: layout, assets: TouchLayoutAssets(images: images))
    }
    return try await withTaskCancellationHandler {
      let result = try await task.value
      try Task.checkCancellation()
      return result
    } onCancel: {
      task.cancel()
    }
  }
}

struct SavedTouchLayoutGroup: Identifiable {
  let id: String
  let name: String
  let layouts: [XboxSavedTouchLayout]
  var lastUsedAt: UInt64 { layouts.map(\.lastUsedAtMs).max() ?? 0 }
}
