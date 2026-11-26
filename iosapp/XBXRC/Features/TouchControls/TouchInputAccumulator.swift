import CoreGraphics
import Foundation

struct TouchPoint: Codable, Equatable, Sendable {
  let x: Double
  let y: Double
  init(_ point: CGPoint) {
    x = point.x
    y = point.y
  }
  var cgPoint: CGPoint { .init(x: x, y: y) }
}

enum TouchPointerPhase: String, Codable, Sendable { case began, moved, ended, cancelled }

struct TouchPointerSample {
  let id: Int
  let phase: TouchPointerPhase
  let position: CGPoint
  var pressure: Double = 0
  var sourceTimestamp: TimeInterval? = nil
  var isCoalesced = false
}

struct TouchPointerEvent: Codable, Equatable, Sendable {
  let pointerID: Int
  let phase: TouchPointerPhase
  let position: TouchPoint
  let delta: TouchPoint
  let pressure: Double
  let controlID: String?
  var sourceTimestamp: TimeInterval? = nil
  var isCoalesced = false
}

struct TouchGamepadSnapshot: Codable, Equatable, Sendable {
  let buttonMask: UInt16
  let leftThumbX: Float
  let leftThumbY: Float
  let rightThumbX: Float
  let rightThumbY: Float
  let leftTrigger: Float
  let rightTrigger: Float
  var sourceActive: Bool?
  init(_ state: IOSStreamGamepadState, sourceActive: Bool? = nil) {
    self.sourceActive = sourceActive
    buttonMask = state.buttonMask
    leftThumbX = state.leftThumbX
    leftThumbY = state.leftThumbY
    rightThumbX = state.rightThumbX
    rightThumbY = state.rightThumbY
    leftTrigger = state.leftTrigger
    rightTrigger = state.rightTrigger
  }
}

struct TouchInputFrame: Encodable, Sendable {
  let layoutID: String?
  let sequence: UInt64
  let timestamp: TimeInterval
  let stateCompletedTimestamp: TimeInterval
  let processingDuration: TimeInterval
  let reason: String
  let events: [TouchPointerEvent]
  let transitions: [TouchGamepadSnapshot]
  let transitionTimestamps: [TimeInterval]
  let activeControlIDs: [String]
  let activeLayerIDs: [String]
  let pointerCount: Int
  let gamepad: IOSStreamGamepadState

  private enum CodingKeys: String, CodingKey {
    case layoutID, sequence, timestamp, reason, events, transitions
    case stateCompletedTimestamp, processingDuration, transitionTimestamps
    case activeControlIDs, activeLayerIDs, pointerCount, gamepad
  }
  func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: CodingKeys.self)
    try container.encodeIfPresent(layoutID, forKey: .layoutID)
    try container.encode(sequence, forKey: .sequence)
    try container.encode(timestamp, forKey: .timestamp)
    try container.encode(stateCompletedTimestamp, forKey: .stateCompletedTimestamp)
    try container.encode(processingDuration, forKey: .processingDuration)
    try container.encode(reason, forKey: .reason)
    try container.encode(events, forKey: .events)
    try container.encode(transitions, forKey: .transitions)
    try container.encode(transitionTimestamps, forKey: .transitionTimestamps)
    try container.encode(activeControlIDs, forKey: .activeControlIDs)
    try container.encode(activeLayerIDs, forKey: .activeLayerIDs)
    try container.encode(pointerCount, forKey: .pointerCount)
    try container.encode(TouchGamepadSnapshot(gamepad), forKey: .gamepad)
  }
}

@MainActor
final class TouchInputAccumulator {
  nonisolated static let maximumPointers = 12
  private(set) var layout: TouchLayout?
  private(set) var sockets: [TouchControlSocket] = []
  private(set) var runtimes: [String: TouchControlRuntime] = [:]
  private(set) var activeLayers: [String] = []
  private(set) var gamepad: IOSStreamGamepadState = .neutral
  private(set) var sequence: UInt64 = 0
  private(set) var diagnostics = TouchInputDiagnostics()
  var onInput: ((TouchInputFrame) -> Void)?
  var onVisualChange: ((TouchInputFrame?) -> Void)?
  private var size = CGSize.zero
  private var insets = TouchSurfaceInsets()
  private var configuration: TouchLayoutConfiguration?
  private var positions: [Int: CGPoint] = [:]
  private var attachedControls: [Int: String] = [:]
  private var sourceTimestamps: [Int: TimeInterval] = [:]
  private var orderedRuntimeIDs: [String] = []
  private let clock: () -> TimeInterval

  init(clock: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }) {
    self.clock = clock
  }

  func resetDiagnostics() { diagnostics = .init() }

  func didRender(at timestamp: TimeInterval) { diagnostics.rendered(at: timestamp) }

  var needsTimeUpdate: Bool { runtimes.values.contains { $0.needsTimeUpdate } }

  func advance(at timestamp: TimeInterval) {
    guard timestamp.isFinite else { return }
    let startedAt = clock()
    var changed = false
    for id in orderedRuntimeIDs {
      if runtimes[id]?.advance(to: timestamp) == true { changed = true }
    }
    guard changed else { return }
    let previousState = gamepad
    gamepad = aggregate()
    let completedAt = clock()
    let transitions: [TouchGamepadSnapshot] =
      previousState == gamepad
      ? [] : [.init(gamepad, sourceActive: !positions.isEmpty || gamepad != .neutral)]
    if previousState != gamepad { diagnostics.stateChanged(at: completedAt) }
    publish(
      events: [], transitions: transitions,
      transitionTimestamps: transitions.isEmpty ? [] : [completedAt], reason: "pointer",
      timestamp: timestamp, completedAt: completedAt,
      processingDuration: completedAt - startedAt)
  }

  func load(_ layout: TouchLayout) {
    cancelAll(reason: "layoutChanged")
    self.layout = layout
    rebuild()
    onVisualChange?(nil)
  }

  /// 动态 Context 只替换变化的控件，其他多指与 sticky 状态继续沿用。
  func updateLayout(_ layout: TouchLayout) {
    let previousState = gamepad
    self.layout = layout
    rebuild()
    updateLayers()
    gamepad = aggregate()
    let now = clock()
    let transitions: [TouchGamepadSnapshot] =
      previousState == gamepad
      ? [] : [.init(gamepad, sourceActive: !positions.isEmpty || gamepad != .neutral)]
    publish(
      events: [], transitions: transitions,
      transitionTimestamps: transitions.isEmpty ? [] : [now], reason: "contextChanged",
      timestamp: now, completedAt: now, processingDuration: 0)
  }

  func setViewport(size: CGSize, insets: TouchSurfaceInsets = .init()) {
    guard size != self.size || insets != self.insets else { return }
    let wasActive = !positions.isEmpty || gamepad != .neutral || !activeLayers.isEmpty
    if wasActive { cancelAll(reason: "viewportChanged") }
    self.size = size
    self.insets = insets
    rebuild()
    onVisualChange?(nil)
  }

  func setConfiguration(_ configuration: TouchLayoutConfiguration?) {
    guard self.configuration != configuration else { return }
    cancelAll(reason: "configurationChanged")
    self.configuration = configuration
    rebuild()
    onVisualChange?(nil)
  }

  func process(
    _ samples: [TouchPointerSample],
    timestamp: TimeInterval? = nil
  ) {
    guard !samples.isEmpty else { return }
    let startedAt = clock()
    let receivedAt = timestamp ?? startedAt
    diagnostics.receive(sampleCount: samples.count, at: receivedAt)
    var events: [TouchPointerEvent] = []
    var transitions: [TouchGamepadSnapshot] = []
    var transitionTimestamps: [TimeInterval] = []
    for var sample in samples {
      let previousPosition = positions[sample.id]
      let terminal = sample.phase == .ended || sample.phase == .cancelled
      guard sample.position.x.isFinite, sample.position.y.isFinite else {
        diagnostics.invalidSamples += 1
        // 终止事件沿上一位置释放，异常坐标同样能够清空输入。
        if terminal, let previousPosition {
          sample = .init(
            id: sample.id, phase: sample.phase, position: previousPosition,
            sourceTimestamp: sample.sourceTimestamp, isCoalesced: sample.isCoalesced)
        } else {
          continue
        }
        processSample(
          sample, receivedAt: receivedAt, events: &events,
          transitions: &transitions, transitionTimestamps: &transitionTimestamps)
        continue
      }
      processSample(
        sample, receivedAt: receivedAt, events: &events,
        transitions: &transitions, transitionTimestamps: &transitionTimestamps)
    }
    let completedAt = clock()
    diagnostics.processed(duration: completedAt - startedAt)
    guard !events.isEmpty else { return }
    publish(
      events: events, transitions: transitions, transitionTimestamps: transitionTimestamps,
      reason: "pointer", timestamp: receivedAt, completedAt: completedAt,
      processingDuration: completedAt - startedAt)
  }

  private func processSample(
    _ sample: TouchPointerSample, receivedAt: TimeInterval,
    events: inout [TouchPointerEvent], transitions: inout [TouchGamepadSnapshot],
    transitionTimestamps: inout [TimeInterval]
  ) {
    let previousPosition = positions[sample.id]
    var source = sample.sourceTimestamp
    if let time = source, !time.isFinite || time < 0 || time > receivedAt {
      diagnostics.invalidSamples += 1
      source = nil
    }
    if let time = source, let previous = sourceTimestamps[sample.id] {
      if time < previous {
        diagnostics.outOfOrderSamples += 1
        if sample.phase == .moved { return }
      } else if time == previous, sample.phase == .moved {
        diagnostics.duplicateSamples += 1
        return
      }
    }
    let previousState = gamepad
    var eventControl = attachedControls[sample.id]
    switch sample.phase {
    case .began:
      guard positions[sample.id] == nil else {
        diagnostics.duplicateSamples += 1
        return
      }
      guard positions.count < Self.maximumPointers else {
        diagnostics.pointerLimitSamples += 1
        return
      }
      positions[sample.id] = sample.position
      attach(sample.id, at: sample.position, timestamp: source ?? receivedAt)
      eventControl = attachedControls[sample.id]
    case .moved:
      guard positions[sample.id] != nil else {
        diagnostics.orphanSamples += 1
        return
      }
      positions[sample.id] = sample.position
      move(sample.id, to: sample.position, timestamp: source ?? receivedAt)
      eventControl = attachedControls[sample.id]
    case .ended, .cancelled:
      guard positions[sample.id] != nil else {
        diagnostics.orphanSamples += 1
        return
      }
      detach(sample.id, cancelled: sample.phase == .cancelled)
      positions.removeValue(forKey: sample.id)
    }
    if sample.phase == .ended || sample.phase == .cancelled {
      sourceTimestamps.removeValue(forKey: sample.id)
    } else if let source {
      sourceTimestamps[sample.id] = source
    }
    updateLayers()
    gamepad = aggregate()
    let stateCompletedAt = clock()
    if gamepad != previousState {
      transitions.append(.init(gamepad, sourceActive: !positions.isEmpty || gamepad != .neutral))
      transitionTimestamps.append(stateCompletedAt)
      diagnostics.stateChanged(at: stateCompletedAt)
    }
    let delta =
      previousPosition.map {
        CGPoint(x: sample.position.x - $0.x, y: sample.position.y - $0.y)
      } ?? .zero
    let event = TouchPointerEvent(
      pointerID: sample.id, phase: sample.phase, position: .init(sample.position),
      delta: .init(delta),
      pressure: min(1, max(0, sample.pressure.isFinite ? sample.pressure : 0)),
      controlID: eventControl, sourceTimestamp: source, isCoalesced: sample.isCoalesced)
    events.append(event)
    diagnostics.observe(event, receivedAt: receivedAt, stateCompletedAt: stateCompletedAt)
  }

  func cancelAll(reason: String = "cancelled") {
    let startedAt = clock()
    let previousState = gamepad
    let events = positions.keys.sorted().map {
      TouchPointerEvent(
        pointerID: $0, phase: .cancelled, position: .init(positions[$0]!),
        delta: .init(.zero), pressure: 0, controlID: attachedControls[$0])
    }
    positions.removeAll()
    sourceTimestamps.removeAll()
    attachedControls.removeAll()
    for key in runtimes.keys { runtimes[key]?.reset() }
    activeLayers.removeAll()
    gamepad = .neutral
    rebuild()
    let completedAt = clock()
    for event in events {
      diagnostics.observe(
        event, receivedAt: startedAt, stateCompletedAt: completedAt,
        synthesized: true)
    }
    if previousState != .neutral { diagnostics.stateChanged(at: completedAt) }
    publish(
      events: events, transitions: [.init(.neutral)], transitionTimestamps: [completedAt],
      reason: reason, timestamp: startedAt, completedAt: completedAt,
      processingDuration: completedAt - startedAt)
  }

  private func attach(_ id: Int, at point: CGPoint, timestamp: TimeInterval) {
    let candidates = sockets.filter {
      $0.control.visible && $0.contains(point)
        && !(runtimes[$0.id]?.hasExclusivePointer ?? false)
    }
    guard let socket = TouchLayoutGeometry.hitTest(point, sockets: candidates),
      runtimes[socket.id]?.begin(pointerID: id, position: point, timestamp: timestamp) == true
    else { return }
    attachedControls[id] = socket.id
  }

  private func detach(_ id: Int, cancelled: Bool) {
    guard let control = attachedControls.removeValue(forKey: id) else { return }
    runtimes[control]?.end(pointerID: id, cancelled: cancelled)
  }

  private func move(_ id: Int, to point: CGPoint, timestamp: TimeInterval) {
    if let control = attachedControls[id], let runtime = runtimes[control] {
      if runtime.socket.control.allowsRollover {
        let next = TouchLayoutGeometry.hitTest(point, sockets: sockets)
        if next?.id != control {
          if next == nil || next?.control.allowsRollover == true {
            detach(id, cancelled: false)
            attach(id, at: point, timestamp: timestamp)
            return
          }
        }
      }
      runtimes[control]?.move(pointerID: id, position: point, timestamp: timestamp)
    } else {
      attach(id, at: point, timestamp: timestamp)
    }
  }

  private func aggregate() -> IOSStreamGamepadState {
    TouchGamepadMapping.state(
      contributions: orderedRuntimeIDs.compactMap { runtimes[$0]?.contribution() })
  }

  private func updateLayers() {
    guard let layout else { return }
    // 控件替换可能释放 layer 的最后来源，有限次重算使这一批输入收敛。
    for _ in 0..<8 {
      let requested = Set(runtimes.values.flatMap { $0.contribution().layers })
        .filter { layout.content["layers"]?[$0] != nil }
      let next =
        activeLayers.filter { requested.contains($0) }
        + requested.filter { !activeLayers.contains($0) }.sorted()
      guard next != activeLayers else { return }
      activeLayers = next
      rebuild()
    }
    // 自引用层的覆盖循环回到基础布局，输入状态保持可释放。
    for key in runtimes.keys { runtimes[key]?.reset() }
    activeLayers.removeAll()
    attachedControls.removeAll()
    rebuild()
  }

  private func rebuild() {
    guard let layout else {
      sockets = []
      runtimes = [:]
      orderedRuntimeIDs = []
      return
    }
    let content = TouchLayoutResolver.resolve(layout, activeLayers: activeLayers)
    sockets = TouchLayoutGeometry.sockets(
      content: content, size: size, insets: insets, compiledControls: layout.compiledControls)
    if let configuration {
      sockets = TouchLayoutGeometry.configured(
        sockets, size: size, insets: insets,
        configuration: configuration)
    }
    var next: [String: TouchControlRuntime] = [:]
    for socket in sockets {
      if var existing = runtimes[socket.id], existing.socket.control == socket.control {
        existing.socket = socket
        next[socket.id] = existing
      } else {
        next[socket.id] = .init(socket: socket)
      }
    }
    attachedControls = attachedControls.filter { id, control in
      next[control]?.pointers[id] != nil
    }
    runtimes = next
    orderedRuntimeIDs = runtimes.keys.sorted()
  }

  private func publish(
    events: [TouchPointerEvent], transitions: [TouchGamepadSnapshot],
    transitionTimestamps: [TimeInterval], reason: String, timestamp: TimeInterval,
    completedAt: TimeInterval, processingDuration: TimeInterval
  ) {
    sequence &+= 1
    let frame = TouchInputFrame(
      layoutID: layout?.id, sequence: sequence, timestamp: timestamp,
      stateCompletedTimestamp: completedAt, processingDuration: processingDuration,
      reason: reason,
      events: events,
      transitions: transitions,
      transitionTimestamps: transitionTimestamps,
      activeControlIDs: runtimes.keys.filter { runtimes[$0]?.isActive == true }.sorted(),
      activeLayerIDs: activeLayers, pointerCount: positions.count, gamepad: gamepad)
    onVisualChange?(frame)
    onInput?(frame)
  }
}
