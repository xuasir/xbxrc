import Foundation
@preconcurrency import GameController

/// 控件目录在 MainActor 建立一次；input 的采样与 handler 生命周期归 InputQueue。
struct StreamingControllerLease: @unchecked Sendable {
    let controller: GCController
    let identifier: String
    let extraButtons: [String]
    let extraAxes: [String]

    @MainActor init(controller: GCController) {
        self.controller = controller
        identifier = StreamInputSession.controllerID(controller)
        let pad = controller.extendedGamepad
        let buttons: [GCControllerButtonInput?] = [
            pad?.buttonA, pad?.buttonB, pad?.buttonX, pad?.buttonY, pad?.buttonMenu,
            pad?.buttonOptions, pad?.buttonHome, pad?.leftShoulder, pad?.rightShoulder,
            pad?.leftTrigger, pad?.rightTrigger, pad?.leftThumbstickButton,
            pad?.rightThumbstickButton, pad?.dpad.up, pad?.dpad.down, pad?.dpad.left,
            pad?.dpad.right,
        ]
        let standardButtons = Set(buttons.compactMap { $0 }.map(ObjectIdentifier.init))
        extraButtons = controller.physicalInputProfile.buttons
            .filter { !standardButtons.contains(ObjectIdentifier($0.value)) }.keys.sorted()
        let axes: [GCControllerAxisInput?] = [
            pad?.leftThumbstick.xAxis, pad?.leftThumbstick.yAxis,
            pad?.rightThumbstick.xAxis, pad?.rightThumbstick.yAxis, pad?.dpad.xAxis,
            pad?.dpad.yAxis,
        ]
        let standardAxes = Set(axes.compactMap { $0 }.map(ObjectIdentifier.init))
        extraAxes = controller.physicalInputProfile.axes
            .filter { !standardAxes.contains(ObjectIdentifier($0.value)) }.keys.sorted()
    }
}

final class StreamingControllerInputDriver: Sendable {
    private let queue: DispatchQueue
    private let sink: @Sendable (StreamingControllerSample, Bool) -> Void
    // 下列状态由 queue 单写，入口均在同一 InputQueue 上调用。
    private nonisolated(unsafe) var lease: StreamingControllerLease?
    private nonisolated(unsafe) var hasBufferedState = false
    private nonisolated(unsafe) var revision: UInt64 = 0
    private nonisolated(unsafe) var disconnectObserver: NSObjectProtocol?

    init(queue: DispatchQueue, sink: @escaping @Sendable (StreamingControllerSample, Bool) -> Void)
    {
        self.queue = queue
        self.sink = sink
        disconnectObserver = NotificationCenter.default.addObserver(
            forName: .GCControllerDidDisconnect, object: nil, queue: nil
        ) { [weak self] notification in
            guard let controller = notification.object as? GCController else { return }
            let identifier = String(ObjectIdentifier(controller).hashValue)
            self?.queue.async { [weak self] in
                guard let self, self.lease?.identifier == identifier else { return }
                self.detach()
                self.sink(.init(), true)
            }
        }
    }

    deinit {
        if let disconnectObserver { NotificationCenter.default.removeObserver(disconnectObserver) }
    }

    func attach(_ next: StreamingControllerLease?) {
        dispatchPrecondition(condition: .onQueue(queue))
        guard lease?.identifier != next?.identifier else { return }
        lease?.controller.input.inputStateAvailableHandler = nil
        revision &+= 1
        hasBufferedState = false
        lease = next
        guard let next else {
            sink(.init(), false)
            return
        }
        let input = next.controller.input
        input.queue = queue
        input.inputStateQueueDepth = 20
        let currentRevision = revision
        input.inputStateAvailableHandler = { [weak self] _ in
            guard let self, self.revision == currentRevision else { return }
            self.drain()
        }
        sink(read(input.capture(), lease: next), false)
        drain()
    }

    func detach() {
        dispatchPrecondition(condition: .onQueue(queue))
        lease?.controller.input.inputStateAvailableHandler = nil
        lease = nil
        revision &+= 1
    }

    private func drain() {
        dispatchPrecondition(condition: .onQueue(queue))
        guard let lease else { return }
        while let state = lease.controller.input.nextInputState() {
            let lostContinuity = hasBufferedState && state.changedElements() == nil
            hasBufferedState = true
            sink(read(state, lease: lease), lostContinuity)
        }
    }

    private func read(_ state: any GCDevicePhysicalInputState, lease: StreamingControllerLease)
        -> StreamingControllerSample
    {
        func button(_ name: String) -> Float {
            state.buttons[GCButtonElementName(rawValue: name)]?.pressedInput.value ?? 0
        }
        func stick(_ name: String) -> (Float, Float) {
            guard let pad = state.dpads[GCDirectionPadElementName(rawValue: name)] else {
                return (0, 0)
            }
            return (pad.xAxis.value, pad.yAxis.value)
        }
        var raw = IOSStreamGamepadState.neutral
        let names: [(String, UInt16)] = [
            ("Button Home", 2), ("Button Menu", 4), ("Button Options", 8),
            ("Button A", 16), ("Button B", 32), ("Button X", 64), ("Button Y", 128),
            ("Left Shoulder", 4096), ("Right Shoulder", 8192),
            ("Left Thumbstick Button", 16384), ("Right Thumbstick Button", 32768),
        ]
        for (name, mask) in names where button(name) > 0.5 { raw.buttonMask |= mask }
        if let dpad = state.dpads[.directionPad] {
            if dpad.up.value > 0.5 { raw.buttonMask |= 256 }
            if dpad.down.value > 0.5 { raw.buttonMask |= 512 }
            if dpad.left.value > 0.5 { raw.buttonMask |= 1024 }
            if dpad.right.value > 0.5 { raw.buttonMask |= 2048 }
        }
        (raw.leftThumbX, raw.leftThumbY) = stick("Left Thumbstick")
        (raw.rightThumbX, raw.rightThumbY) = stick("Right Thumbstick")
        raw.leftTrigger = button("Left Trigger")
        raw.rightTrigger = button("Right Trigger")
        return .init(
            raw: raw,
            buttons: Dictionary(
                uniqueKeysWithValues: lease.extraButtons.map { ("extra:" + $0, button($0)) }),
            axes: Dictionary(
                uniqueKeysWithValues: lease.extraAxes.map {
                    (
                        "axis:" + $0,
                        state.axes[GCAxisElementName(rawValue: $0)]?.absoluteInput?.value ?? 0
                    )
                }), connected: true,
            collectedAtMs: StreamingFrameFeedbackClock.nowMilliseconds
                - max(0, state.lastEventLatency * 1_000))
    }
}
