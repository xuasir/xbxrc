import { KeyboardFrame, MouseFrame, PointerFrame } from '../../domain/input'

export class PointerInputController {
    private mouseActive = false
    private mouseLocked = false
    private touchActive = false
    private mouseStateButtons = 0
    private touchEvents: Record<number, { events: Array<any> }> = {}
    private keyboardButtonsState: Record<number, boolean> = {}

    constructor(private readonly getMouseSensitivity: () => number, private readonly isMouseKeyboardEnabled: () => boolean) {}

    flushPointerFrames(): Array<PointerFrame> {
        const result = Object.keys(this.touchEvents).map((key) => ({ events: this.touchEvents[Number(key)].events }))
        this.touchEvents = {}
        return result
    }

    onPointerMove(e: PointerEvent, queueMouse: (frame: MouseFrame) => void): void {
        e.preventDefault()
        if (this.mouseActive && this.mouseLocked) {
            queueMouse({
                X: e.movementX * this.getMouseSensitivity(),
                Y: e.movementY * this.getMouseSensitivity(),
                WheelX: 0,
                WheelY: 0,
                Buttons: e.buttons,
                Relative: 0,
            })
        }
        if (this.touchActive) {
            if (!this.touchEvents[e.pointerId]) {
                this.touchEvents[e.pointerId] = { events: [] }
            }
            this.touchEvents[e.pointerId].events.push(e)
        }
    }

    onPointerDownOrUp(e: PointerEvent, queueMouse: (frame: MouseFrame) => void): void {
        e.preventDefault()
        if (e.pointerType === 'touch') {
            this.mouseActive = false
            this.touchActive = true
        } else if (e.pointerType === 'mouse') {
            this.mouseActive = true
            this.touchActive = false
        }
        if (this.isMouseKeyboardEnabled() && this.mouseActive && !this.mouseLocked) {
            this.requestPointerLockWithUnadjustedMovement(e.target as HTMLElement)
            return
        }
        if (this.mouseActive && this.mouseLocked) {
            this.mouseStateButtons = e.buttons
            queueMouse({
                X: e.movementX * this.getMouseSensitivity(),
                Y: e.movementY * this.getMouseSensitivity(),
                WheelX: 0,
                WheelY: 0,
                Buttons: this.mouseStateButtons,
                Relative: 0,
            })
        }
        window.setTimeout(() => {
            if (!this.touchActive) {
                return
            }
            if (!this.touchEvents[e.pointerId]) {
                this.touchEvents[e.pointerId] = { events: [] }
            }
            this.touchEvents[e.pointerId].events.push(e)
        }, 32)
    }

    onWheel(e: WheelEvent): void {
        e.preventDefault()
    }

    onKeyboardPointerLockedDown(event: KeyboardEvent, queueKeyboard: (frame: KeyboardFrame) => void): void {
        if (!this.mouseActive || !this.mouseLocked || this.keyboardButtonsState[event.keyCode]) {
            return
        }
        this.keyboardButtonsState[event.keyCode] = true
        queueKeyboard({ pressed: true, key: event.key, keyCode: event.keyCode })
        window.setTimeout(() => {
            queueKeyboard({ pressed: true, key: event.key, keyCode: event.keyCode })
        }, 16)
    }

    onKeyboardPointerLockedUp(event: KeyboardEvent, queueKeyboard: (frame: KeyboardFrame) => void): void {
        if (!this.mouseActive || !this.mouseLocked) {
            return
        }
        this.keyboardButtonsState[event.keyCode] = false
        queueKeyboard({ pressed: false, key: event.key, keyCode: event.keyCode })
        window.setTimeout(() => {
            queueKeyboard({ pressed: false, key: event.key, keyCode: event.keyCode })
        }, 16)
    }

    private requestPointerLockWithUnadjustedMovement(element: HTMLElement): Promise<void> {
        const promise = (element as any).requestPointerLock({ unadjustedMovement: true })
        if ('keyboard' in navigator && 'lock' in (navigator.keyboard as any)) {
            document.body.requestFullscreen().then(() => (navigator as any).keyboard.lock()).catch(() => undefined)
        }
        return promise
            .then(() => {
                this.mouseLocked = true
                document.addEventListener('pointerlockchange', () => {
                    this.mouseLocked = document.pointerLockElement !== null
                }, false)
            })
            .catch((error: any) => {
                if (error.name === 'NotSupportedError') {
                    this.mouseLocked = true
                    return (element as any).requestPointerLock()
                }
                throw error
            })
    }
}
