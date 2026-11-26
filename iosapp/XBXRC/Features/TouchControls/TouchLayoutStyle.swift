import Foundation
import UIKit

/// 官方颜色引用共用默认/高对比度色板；布局声明优先于系统颜色。
enum TouchStylePalette {
  static let system: [String: String] = TouchProtocolCatalog.information.systemColors

  static func hex(_ value: String?, palette: TouchJSON, highContrast: Bool = false) -> String? {
    guard let value else { return nil }
    if value.hasPrefix("#") {
      let digits = value.dropFirst()
      guard [3, 4, 6, 8].contains(digits.count), UInt32(digits, radix: 16) != nil else {
        return nil
      }
      return value
    }
    guard value.hasPrefix("colors/") else { return nil }
    let name = String(value.dropFirst(7))
    let color =
      (highContrast ? palette["highContrast"]?[name]?.string : nil)
      ?? palette["default"]?[name]?.string ?? system[name]
    guard let color, color.hasPrefix("#") else { return nil }
    return hex(color, palette: .object([:]))
  }

  static func color(_ value: String?, palette: TouchJSON, highContrast: Bool = false) -> UIColor? {
    guard let hex = hex(value, palette: palette, highContrast: highContrast) else { return nil }
    var digits = String(hex.dropFirst())
    if digits.count == 3 || digits.count == 4 { digits = String(digits.flatMap { [$0, $0] }) }
    guard let value = UInt32(digits, radix: 16) else { return nil }
    let rgb = digits.count == 8 ? value >> 8 : value
    return UIColor(
      red: CGFloat((rgb >> 16) & 255) / 255,
      green: CGFloat((rgb >> 8) & 255) / 255, blue: CGFloat(rgb & 255) / 255,
      alpha: digits.count == 8 ? CGFloat(value & 255) / 255 : 1)
  }
}
