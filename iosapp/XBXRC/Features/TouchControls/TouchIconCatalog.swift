import Foundation
import UIKit

/// 官方 v4.1 的全部图标名称映射到 iOS 原生语义符号，同类变体共用图形。
enum TouchIconCatalog {
  /// 系统缺少的武器/拳头使用本地向量轮廓，随现有符号缓存复用。
  @MainActor
  static func artwork(_ symbol: String, pointSize: CGFloat = 64) -> UIImage? {
    guard ["sword", "pickaxe", "hand.fist"].contains(symbol) else {
      return UIImage(
        systemName: symbol,
        withConfiguration: UIImage.SymbolConfiguration(pointSize: pointSize, weight: .regular))
    }
    return UIGraphicsImageRenderer(size: CGSize(width: 64, height: 64)).image { _ in
      UIColor.white.setStroke()
      UIColor.white.setFill()
      let path = UIBezierPath()
      path.lineWidth = 3.5
      path.lineCapStyle = .round
      path.lineJoinStyle = .round
      switch symbol {
      case "sword":
        path.move(to: CGPoint(x: 24, y: 42))
        path.addLine(to: CGPoint(x: 48, y: 14))
        path.addLine(to: CGPoint(x: 53, y: 10))
        path.addLine(to: CGPoint(x: 52, y: 18))
        path.addLine(to: CGPoint(x: 29, y: 46))
        path.close()
        path.stroke()
        path.removeAllPoints()
        path.move(to: CGPoint(x: 16, y: 36))
        path.addLine(to: CGPoint(x: 36, y: 53))
        path.move(to: CGPoint(x: 25, y: 45))
        path.addLine(to: CGPoint(x: 15, y: 56))
        path.stroke()
      case "pickaxe":
        path.move(to: CGPoint(x: 16, y: 54))
        path.addLine(to: CGPoint(x: 43, y: 21))
        path.move(to: CGPoint(x: 15, y: 23))
        path.addQuadCurve(to: CGPoint(x: 53, y: 41), controlPoint: CGPoint(x: 42, y: 10))
        path.stroke()
      default:
        path.move(to: CGPoint(x: 19, y: 48))
        path.addLine(to: CGPoint(x: 15, y: 30))
        path.addQuadCurve(to: CGPoint(x: 20, y: 17), controlPoint: CGPoint(x: 12, y: 17))
        path.addLine(to: CGPoint(x: 43, y: 17))
        path.addQuadCurve(to: CGPoint(x: 50, y: 25), controlPoint: CGPoint(x: 52, y: 17))
        path.addLine(to: CGPoint(x: 49, y: 40))
        path.addLine(to: CGPoint(x: 42, y: 48))
        path.close()
        path.stroke()
        path.removeAllPoints()
        for x in [24, 32, 40] {
          path.move(to: CGPoint(x: CGFloat(x), y: 18))
          path.addLine(to: CGPoint(x: CGFloat(x), y: 28))
        }
        path.move(to: CGPoint(x: 17, y: 32))
        path.addLine(to: CGPoint(x: 36, y: 32))
        path.stroke()
      }
    }
  }
  static let symbols: [String: String] = [
    "ability": "sparkles",
    "ability2": "sparkles",
    "ability3": "sparkles",
    "abilityPowerPunch": "hand.fist",
    "abilityPowerUp": "bolt",
    "accept": "checkmark",
    "add": "plus",
    "aim": "scope",
    "armor": "shield.lefthalf.filled",
    "arrow": "arrow.up.right",
    "arrowReload": "arrow.clockwise",
    "attackBehind": "arrow.uturn.backward",
    "barrel": "cylinder",
    "block": "shield",
    "bomb": "burst",
    "book": "book",
    "bow": "arrow.up.right",
    "brakePedal": "pedal.brake",
    "brightness": "sun.max",
    "capture": "camera",
    "character": "person",
    "characterSelect": "person.crop.circle",
    "characterSelect2": "person.crop.circle",
    "chat": "bubble.left",
    "climbStairs": "stairs",
    "close": "xmark",
    "compass": "safari",
    "cover": "shield",
    "crouch": "figure.flexibility",
    "cursor": "cursorarrow",
    "dPad": "dpad",
    "dash": "figure.run",
    "defendByShield": "shield",
    "dodge": "arrow.turn.up.right",
    "downArrow": "arrow.down",
    "downArrow2": "arrow.down",
    "downChevron": "chevron.down",
    "emotes": "face.smiling",
    "enterCar": "car",
    "enterDoor": "door.left.hand.open",
    "exit": "rectangle.portrait.and.arrow.right",
    "exitCar": "car",
    "exitDoor": "door.right.hand.open",
    "fastForward": "forward",
    "fire": "scope",
    "firePunch": "hand.fist",
    "flag": "flag",
    "gasPedal": "pedal.accelerator",
    "glide": "paperplane",
    "golf": "figure.golf",
    "grab": "hand.raised",
    "grenade": "burst",
    "gyroscope": "gyroscope",
    "handbrake": "parkingsign.circle",
    "handbrake2": "parkingsign.circle",
    "health": "heart",
    "heavyKick": "figure.kickboxing",
    "heavyKick2": "figure.kickboxing",
    "heavyKick3": "figure.kickboxing",
    "heavyKick4": "figure.kickboxing",
    "heavyPunch": "hand.fist",
    "heavyPunch2": "hand.fist",
    "heavyPunch3": "hand.fist",
    "heavySword": "sword",
    "heavySword2": "sword",
    "help": "questionmark",
    "horn": "speaker.wave.3",
    "hourglass": "hourglass",
    "interact": "hand.tap",
    "internet": "globe",
    "inventory": "backpack",
    "jump": "arrow.up",
    "kick": "figure.kickboxing",
    "largeGridView": "square.grid.2x2",
    "leftArrow": "arrow.left",
    "leftArrow2": "arrow.left",
    "leftChevron": "chevron.left",
    "leftRightArrows": "arrow.left.arrow.right",
    "lightKick": "figure.kickboxing",
    "lightKick2": "figure.kickboxing",
    "lightKick3": "figure.kickboxing",
    "lightKick4": "figure.kickboxing",
    "lightPunch": "hand.fist",
    "lightPunch2": "hand.fist",
    "lightPunch3": "hand.fist",
    "lightSword": "sword",
    "lightSword2": "sword",
    "look": "eye",
    "lookBehind": "arrow.uturn.backward",
    "lookBehind2": "arrow.uturn.backward",
    "lookByHand": "hand.raised",
    "map": "map",
    "map2": "map",
    "medical": "cross.case",
    "meditate": "figure.mind.and.body",
    "mediumKick": "figure.kickboxing",
    "mediumKick2": "figure.kickboxing",
    "mediumKick3": "figure.kickboxing",
    "mediumKick4": "figure.kickboxing",
    "mediumPunch": "hand.fist",
    "mediumPunch2": "hand.fist",
    "mediumPunch3": "hand.fist",
    "mediumSword": "sword",
    "mediumSword2": "sword",
    "microphone": "mic",
    "mirror": "rectangle.on.rectangle",
    "moreActions": "ellipsis",
    "move": "arrow.up.and.down.and.arrow.left.and.right",
    "move2": "arrow.up.and.down.and.arrow.left.and.right",
    "notebook": "book.closed",
    "parameters": "slider.horizontal.3",
    "pause": "pause",
    "phone": "phone",
    "pickAxe": "pickaxe",
    "placeholder": "circle.dotted",
    "plane": "airplane",
    "planeFast": "airplane",
    "planeSlow": "airplane",
    "punch": "hand.fist",
    "punch2": "hand.fist",
    "radialMenu": "circle.grid.3x3",
    "radialMenu2": "circle.grid.3x3",
    "radio": "radio",
    "ram": "bolt",
    "redo": "arrow.uturn.forward",
    "reload": "arrow.clockwise",
    "repeatRefresh": "arrow.counterclockwise",
    "reset": "arrow.counterclockwise",
    "rewind": "backward",
    "rightArrow": "arrow.right",
    "rightArrow2": "arrow.right",
    "rightChevron": "chevron.right",
    "roll": "arrow.trianglehead.2.clockwise.rotate.90",
    "run": "figure.run",
    "select": "checkmark.circle",
    "selectAll": "checkmark.circle.fill",
    "selectionWheel": "circle.grid.3x3",
    "sit": "chair",
    "skateboard": "figure.skating",
    "skateboardGrab": "hand.raised",
    "skateboardGrind": "figure.skating",
    "skateboardJump": "arrow.up",
    "skateboardOllie": "arrow.up.forward",
    "skateboardRampOver": "arrow.up.right",
    "slide": "arrow.down.right",
    "smallGridView": "square.grid.3x3",
    "speaker": "speaker.wave.2",
    "specialAbility": "sparkles",
    "sprint": "figure.run",
    "stealth": "eye.slash",
    "steering": "steeringwheel",
    "stopwatch": "stopwatch",
    "subtract": "minus",
    "surf": "figure.surfing",
    "switchCamera": "camera.rotate",
    "sword": "sword",
    "sword2": "sword",
    "sync": "arrow.triangle.2.circlepath",
    "targetLock": "scope",
    "team": "person.3",
    "teamAttack": "person.3.sequence",
    "throw": "figure.handball",
    "titleMenu": "line.3.horizontal",
    "touch": "hand.tap",
    "undo": "arrow.uturn.backward",
    "upArrow": "arrow.up",
    "upArrow2": "arrow.up",
    "upChevron": "chevron.up",
    "walk": "figure.walk",
    "waypoint": "mappin",
    "weaponSelect": "scope",
    "zoomIn": "plus.magnifyingglass",
    "zoomOut": "minus.magnifyingglass",
    "gamepadA": "a.circle",
    "gamepadB": "b.circle",
    "gamepadX": "x.circle",
    "gamepadY": "y.circle",
  ]
}
