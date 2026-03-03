import { InputFrame, InputRuntimeConfig } from '../../domain/input'
import { TypedEventEmitter } from '../../api/events'
import { NativeBridge } from '../../infra/bridge/NativeBridge'

export class RumbleService {
    private rumbleInterval: Record<number, number | undefined> = { 0: undefined, 1: undefined, 2: undefined, 3: undefined }

    constructor(
    private readonly nativeBridge: NativeBridge,
    private readonly emitter: TypedEventEmitter<any>,
    ) {}

    destroy(): void {
        for (const key of Object.keys(this.rumbleInterval)) {
            const handle = this.rumbleInterval[Number(key)]
            if (handle) {
                window.clearInterval(handle)
                this.rumbleInterval[Number(key)] = undefined
            }
        }
    }

    handlePacket(event: MessageEvent<ArrayBuffer>, runtime: InputRuntimeConfig): void {
        const dataView = new DataView(event.data)
        let offset = 2
        const reportType = dataView.getUint8(0)
        if (reportType !== 128 || !runtime.vibrationEnabled) {
            return
        }
        dataView.getUint8(offset)
        const gamepadIndex = dataView.getUint8(offset + 1)
        offset += 2
        const leftMotorPercent = dataView.getUint8(offset) / 100
        const rightMotorPercent = dataView.getUint8(offset + 1) / 100
        const leftTriggerMotorPercent = dataView.getUint8(offset + 2) / 100
        const rightTriggerMotorPercent = dataView.getUint8(offset + 3) / 100
        const durationMs = dataView.getUint16(offset + 4, true)
        const delayMs = dataView.getUint16(offset + 6, true)
        const repeat = dataView.getUint8(offset + 8)
        const rumbleData = {
            startDelay: 0,
            duration: durationMs / 10,
            weakMagnitude: rightMotorPercent,
            strongMagnitude: leftMotorPercent,
            leftTrigger: leftTriggerMotorPercent,
            rightTrigger: rightTriggerMotorPercent,
        }

        if (runtime.vibrationMode === 'Device') {
            this.nativeBridge.post({ type: 'deviceVibration', message: { rumbleData, repeat } })
            return
        }
        if (runtime.vibrationMode === 'Native') {
            this.nativeBridge.post({ type: 'nativeVibration', message: { rumbleData, repeat } })
            return
        }
        try {
            const gamepad = navigator.getGamepads()[gamepadIndex] as any
            if (!gamepad?.vibrationActuator) {
                return
            }
            gamepad.vibrationActuator.playEffect(
                gamepad.vibrationActuator.effects?.includes('trigger-rumble') && (rumbleData.leftTrigger > 0 || rumbleData.rightTrigger > 0)
                    ? 'trigger-rumble'
                    : gamepad.vibrationActuator.type,
                rumbleData,
            )
            if (repeat > 0) {
                if (this.rumbleInterval[gamepadIndex]) {
                    window.clearInterval(this.rumbleInterval[gamepadIndex])
                }
                let repeatCount = repeat
                this.rumbleInterval[gamepadIndex] = window.setInterval(() => {
                    if (repeatCount <= 0) {
                        const handle = this.rumbleInterval[gamepadIndex]
                        if (handle) {
                            window.clearInterval(handle)
                            this.rumbleInterval[gamepadIndex] = undefined
                        }
                        return
                    }
                    gamepad.vibrationActuator.playEffect(gamepad.vibrationActuator.type, rumbleData)
                    repeatCount--
                }, delayMs + durationMs)
            }
        } catch (error) {
            this.emitter.emit('error', { error })
        }
    }

    applyForcedTriggerRumble(frame: InputFrame, runtime: InputRuntimeConfig): void {
        if (!runtime.forceTriggerRumble) {
            return
        }
        const index = runtime.gamepadIndex > -1 ? runtime.gamepadIndex : 0
        const gamepad = navigator.getGamepads()[index] as any
        if (!gamepad?.vibrationActuator || gamepad.vibrationActuator.type !== 'dual-rumble') {
            return
        }
        if (!gamepad.vibrationActuator.effects?.includes('trigger-rumble')) {
            return
        }
        const play = (left: number, right: number) => {
            gamepad.vibrationActuator.playEffect('trigger-rumble', {
                duration: 50,
                leftTrigger: right,
                rightTrigger: left,
                strongMagnitude: 0,
                weakMagnitude: 1,
            })
        }
        if (runtime.forceTriggerRumble === 'all') {
            if (frame.LeftTrigger > 0.5) {
                play(frame.LeftTrigger, 0)
            }
            if (frame.RightTrigger > 0.5) {
                play(0, frame.RightTrigger)
            }
            return
        }
        if (runtime.forceTriggerRumble === 'left' && frame.LeftTrigger > 0.5) {
            play(frame.LeftTrigger, 0)
            return
        }
        if (runtime.forceTriggerRumble === 'right' && frame.RightTrigger > 0.5) {
            play(0, frame.RightTrigger)
        }
    }
}
