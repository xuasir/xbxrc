import { DEFAULT_INPUT_FRAME, InputFrame, InputRuntimeConfig } from '../../domain/input'
import { KeyboardDriver } from './KeyboardDriver'

const KEYCODE_KEY_N = 'n'

export interface GamepadDriverDelegate {
  onGamepadAdded(index: number): void;
  onGamepadRemoved(index: number): void;
  onFrame(frame: InputFrame): void;
  getRuntimeConfig(): InputRuntimeConfig;
}

export class GamepadDriver {
    private shadowGamepad: InputFrame = DEFAULT_INPUT_FRAME()
    private activeGamepads: Record<number, boolean> = { 0: false, 1: false, 2: false, 3: false }
    private activeGamepadsInterval?: number
    private runTimer?: number
    private nexusOverrideN = false
    private isVirtualButtonPressing = false
    private readonly gamepadMapping: Record<string, string> = {
        A: '0',
        B: '1',
        X: '2',
        Y: '3',
        DPadUp: '12',
        DPadDown: '13',
        DPadLeft: '14',
        DPadRight: '15',
        LeftShoulder: '4',
        RightShoulder: '5',
        LeftThumb: '10',
        RightThumb: '11',
        LeftTrigger: '6',
        RightTrigger: '7',
        Menu: '9',
        View: '8',
        Nexus: '16',
    }

    private readonly axesMapping: Record<string, string> = {
        LeftThumbXAxis: '0',
        LeftThumbYAxis: '1',
        RightThumbXAxis: '2',
        RightThumbYAxis: '3',
    }

    private readonly downFunc = (e: KeyboardEvent) => this.onKeyChange(e, true)
    private readonly upFunc = (e: KeyboardEvent) => this.onKeyChange(e, false)

    constructor(
    private readonly keyboardDriver: KeyboardDriver,
    private readonly delegate: GamepadDriverDelegate,
    ) {}

    start(): void {
        window.addEventListener('keydown', this.downFunc)
        window.addEventListener('keyup', this.upFunc)
        this.activeGamepads = { 0: false, 1: false, 2: false, 3: false }
        this.activeGamepadsInterval = window.setInterval(() => {
            const gamepads = navigator.getGamepads()
            for (let gamepad = 1; gamepad < gamepads.length; gamepad++) {
                const gamepadState = gamepads[gamepad]
                if (gamepadState === null && this.activeGamepads[gamepad] === true) {
                    this.delegate.onGamepadRemoved(gamepad)
                    this.activeGamepads[gamepad] = false
                    return
                }
                if (gamepadState !== null && this.activeGamepads[gamepad] === false) {
                    this.delegate.onGamepadAdded(gamepad)
                    this.activeGamepads[gamepad] = true
                    return
                }
            }
        }, 500)
        this.run()
    }

    stop(): void {
        if (this.activeGamepadsInterval) {
            window.clearInterval(this.activeGamepadsInterval)
        }
        if (this.runTimer) {
            window.clearTimeout(this.runTimer)
        }
        window.removeEventListener('keydown', this.downFunc)
        window.removeEventListener('keyup', this.upFunc)
    }

    setGamepadState(state: InputFrame): void {
        this.shadowGamepad = state
        this.delegate.onFrame(this.shadowGamepad)
    }

    pressButtonStart(button: keyof InputFrame): void {
        this.isVirtualButtonPressing = true;
        (this.shadowGamepad[button] as any) = 1
        this.delegate.onFrame({ ...this.shadowGamepad })
    }

    pressButtonEnd(button: keyof InputFrame): void {
        (this.shadowGamepad[button] as any) = 0
        this.delegate.onFrame({ ...this.shadowGamepad })
        this.isVirtualButtonPressing = false
    }

    moveLeftStick(x: number, y: number): void {
        this.isVirtualButtonPressing = x !== 0 || y !== 0
        this.shadowGamepad.LeftThumbXAxis = x
        this.shadowGamepad.LeftThumbYAxis = -y
        this.delegate.onFrame({ ...this.shadowGamepad })
    }

    moveRightStick(x: number, y: number): void {
        this.isVirtualButtonPressing = x !== 0 || y !== 0
        this.shadowGamepad.RightThumbXAxis = x
        this.shadowGamepad.RightThumbYAxis = -y
        this.delegate.onFrame({ ...this.shadowGamepad })
    }

    requestStates(): Array<InputFrame> {
        const runtime = this.delegate.getRuntimeConfig()
        const gamepads = navigator.getGamepads()
        if (runtime.gamepadKernel === 'Native') {
            return [((globalThis as any).gpState ?? DEFAULT_INPUT_FRAME()) as InputFrame]
        }
        if (runtime.gamepadIndex > -1) {
            const gamepad = gamepads[runtime.gamepadIndex]
            if (gamepad && gamepad.connected) {
                let state = this.mapStateLabels(gamepad.buttons, gamepad.axes, runtime)
                if (runtime.legacyKeyboard) {
                    state = this.mergeState(state, this.keyboardDriver.requestState())
                }
                return [state]
            }
            return []
        }
        if (runtime.gamepadMix) {
            let merged: InputFrame | null = null
            for (const gamepad of Array.from(gamepads)) {
                if (!gamepad || !gamepad.connected || this.isVirtualController(gamepad)) {
                    continue
                }
                const current = this.mapStateLabels(gamepad.buttons, gamepad.axes, runtime)
                merged = merged ? this.mergeState(merged, current) : current
            }
            if (!merged) {
                merged = DEFAULT_INPUT_FRAME()
            }
            if (runtime.legacyKeyboard) {
                merged = this.mergeState(merged, this.keyboardDriver.requestState())
            }
            return [merged]
        }
        const states: Array<InputFrame> = []
        for (const gamepad of Array.from(gamepads)) {
            if (!gamepad || !gamepad.connected || this.isVirtualController(gamepad)) {
                continue
            }
            let state = this.mapStateLabels(gamepad.buttons, gamepad.axes, runtime)
            if (runtime.legacyKeyboard) {
                state = this.mergeState(state, this.keyboardDriver.requestState())
            }
            states.push(state)
        }
        return states
    }

    run(): void {
        const frames = this.requestStates()
        for (const frame of frames) {
            if (this.nexusOverrideN) {
                frame.Nexus = 1
            }
        }

        if (!this.isVirtualButtonPressing) {
            for (const frame of frames) {
                this.delegate.onFrame(frame)
            }
        }
        this.runTimer = window.setTimeout(() => this.run(), 1000 / this.delegate.getRuntimeConfig().pollingRate)
    }

    private onKeyChange(e: KeyboardEvent, down: boolean): void {
        if (e.key === KEYCODE_KEY_N) {
            this.nexusOverrideN = down
        }
    }

    private mergeState(gpState: InputFrame, kbState: InputFrame): InputFrame {
        const merged = DEFAULT_INPUT_FRAME()
        for (const key of Object.keys(merged) as Array<keyof InputFrame>) {
            const left = gpState[key] as number
            const right = kbState[key] as number
            if (typeof left === 'number' && typeof right === 'number') {
                if (String(key).includes('XAxis') || String(key).includes('YAxis')) {
                    (merged[key] as any) = Math.abs(left) > Math.abs(right) ? left : right
                } else {
                    (merged[key] as any) = Math.max(left, right)
                }
            }
        }
        return merged
    }

    private mapStateLabels(buttons: ReadonlyArray<GamepadButton>, axes: ReadonlyArray<number>, runtime: InputRuntimeConfig): InputFrame {
        const frame = DEFAULT_INPUT_FRAME()
        const mapping = runtime.customGamepadMapping ?? this.gamepadMapping
        for (const button of Object.keys(mapping)) {
            const index = Number(mapping[button])
            if (buttons[index]) {
                (frame as any)[button] = buttons[index].value || 0
            }
        }
        for (const axis of Object.keys(this.axesMapping)) {
            const index = Number(this.axesMapping[axis]);
            (frame as any)[axis] = this.normaliseAxis(axes[index] ?? 0, runtime)
        }
        if (frame.View > 0 && frame.Menu > 0) {
            frame.View = 0
            frame.Menu = 0
            frame.Nexus = 1
        }
        return frame
    }

    private normaliseAxis(value: number, runtime: InputRuntimeConfig): number {
        if (Math.abs(value) < runtime.gamepadDeadZone) {
            return 0
        }
        value = value - Math.sign(value) * runtime.gamepadDeadZone
        value /= 1 - runtime.gamepadDeadZone
        const threshold = 0.8
        const compensation = runtime.edgeCompensation / 100 || 0
        if (Math.abs(value) > threshold) {
            if (value > 0) {
                value = Math.min(value + compensation, 1)
            } else {
                value = Math.max(value - compensation, -1)
            }
        }
        return value
    }

    private isVirtualController(gamepad: Gamepad): boolean {
        return !!gamepad.id && (gamepad.id.includes('virtual') || gamepad.id.includes('Virtual')) && gamepad.axes.length !== 4
    }
}
