import CoreGraphics
import Foundation
import ImageIO
import UIKit

/// 图集按官方帧目录读取，裁剪结果复用；缺失资源交由现有符号或动作标签回退。
@MainActor
enum TouchIconAtlas {
  private struct Rect: Decodable {
    let x: Int
    let y: Int
    let w: Int
    let h: Int
  }
  private struct Frame: Decodable {
    let frame: Rect
    let spriteSourceSize: Rect
    let rotated: Bool
    let sourceSize: Size
  }
  private struct Size: Decodable {
    let w: Int
    let h: Int
  }
  private struct Metadata: Decodable {
    let revision: String
    let dpi: Double
    let frames: [String: Frame]
  }
  private struct Resource {
    let metadata: Metadata
    let image: CGImage
  }
  private static let cache: NSCache<NSString, UIImage> = {
    let cache = NSCache<NSString, UIImage>()
    cache.countLimit = 256
    cache.totalCostLimit = 8 * 1024 * 1024
    return cache
  }()
  private static let resource: Resource? = {
    func url(_ ext: String) -> URL? {
      Bundle.main.url(forResource: "tak-icons", withExtension: ext, subdirectory: "TouchLayouts")
        ?? Bundle.main.url(forResource: "tak-icons", withExtension: ext)
    }
    guard let metadataURL = url("json"), let imageURL = url("png"),
      let data = try? Data(contentsOf: metadataURL),
      let metadata = try? JSONDecoder().decode(Metadata.self, from: data),
      metadata.revision == TouchProtocolCatalog.information.revision,
      let source = CGImageSourceCreateWithURL(imageURL as CFURL, nil),
      let image = CGImageSourceCreateImageAtIndex(source, 0, nil),
      image.width <= 4096, image.height <= 4096
    else { return nil }
    return Resource(metadata: metadata, image: image)
  }()

  static var names: Set<String> { Set(resource?.metadata.frames.keys.map { $0 } ?? []) }

  static func artwork(_ name: String) -> UIImage? {
    if let image = cache.object(forKey: name as NSString) { return image }
    guard let resource, let entry = resource.metadata.frames[name],
      entry.frame.w > 0, entry.frame.h > 0, entry.sourceSize.w > 0, entry.sourceSize.h > 0,
      let cropped = resource.image.cropping(
        to: CGRect(
          x: CGFloat(entry.frame.x), y: CGFloat(entry.frame.y), width: CGFloat(entry.frame.w),
          height: CGFloat(entry.frame.h)))
    else { return nil }
    let format = UIGraphicsImageRendererFormat()
    format.scale = 1
    format.opaque = false
    let size = CGSize(width: CGFloat(entry.sourceSize.w), height: CGFloat(entry.sourceSize.h))
    let result = UIGraphicsImageRenderer(size: size, format: format).image { context in
      let rect = CGRect(
        x: CGFloat(entry.spriteSourceSize.x), y: CGFloat(entry.spriteSourceSize.y),
        width: CGFloat(entry.spriteSourceSize.w), height: CGFloat(entry.spriteSourceSize.h))
      if entry.rotated {
        context.cgContext.translateBy(x: rect.minX, y: rect.maxY)
        context.cgContext.rotate(by: -.pi / 2)
        UIImage(cgImage: cropped).draw(
          in: CGRect(x: 0, y: 0, width: rect.height, height: rect.width))
      } else {
        UIImage(cgImage: cropped).draw(in: rect)
      }
    }
    cache.setObject(
      result, forKey: name as NSString, cost: entry.sourceSize.w * entry.sourceSize.h * 4)
    return result
  }
}
