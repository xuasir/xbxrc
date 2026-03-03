import { TypedEventEmitter } from '../../api/events'
import {
    DEFAULT_INPUT_FRAME,
    InputFrame,
    InputRuntimeConfig,
    KeyboardFrame,
    MouseFrame,
    PointerFrame,
    ProcessedVideoFrameMetadata,
} from '../../domain/input'
import { InputPacketEncoder } from '../../protocol/input/InputPacketEncoder'
import { KeyboardDriver } from '../../infra/input/KeyboardDriver'
import { NativeBridge } from '../../infra/bridge/NativeBridge'
import { PointerInputController } from './PointerInputController'
import { RumbleService } from './RumbleService'

export interface InputDriverLike {
  start(): void;
  stop(): void;
  run?(): void;
  requestStates(): Array<InputFrame>;
  setGamepadState?(frame: InputFrame): void;
  pressButtonStart?(button: keyof InputFrame): void;
  pressButtonEnd?(button: keyof InputFrame): void;
  moveLeftStick?(x: number, y: number): void;
  moveRightStick?(x: number, y: number): void;
}

export interface InputTransport {
  send(data: ArrayBuffer): void;
  getReadyState(): RTCDataChannelState | 'closed';
}

export interface ControlTransport {
  sendGamepadAdded(index: number): void;
  sendGamepadRemoved(index: number): void;
}

export class InputService {
    private inputSequenceNum = 0
    private frameMetadataQueue: Array<ProcessedVideoFrameMetadata> = []
    private gamepadFrames: Array<InputFrame> = []
    private pointerFrames: Array<PointerFrame> = []
    private mouseFrames: Array<MouseFrame> = []
    private keyboardFrames: Array<KeyboardFrame> = []
    private inputInterval?: number
    private isVirtualButtonPressing = false
    private keyboardInputEnabled = true
    private currentInputTransport?: InputTransport
    private currentControlTransport?: ControlTransport
    private readonly pointerController: PointerInputController
    private readonly rumbleService: RumbleService
    private debugInputPacketCount = 0

    readonly keyboardDriver: KeyboardDriver
    readonly gamepadDriver: InputDriverLike

    constructor(
    private runtime: InputRuntimeConfig,
    keyboardDriver: KeyboardDriver,
    gamepadDriver: InputDriverLike,
    private readonly nativeBridge: NativeBridge,
    private readonly emitter: TypedEventEmitter<any>,
    ) {
        this.keyboardDriver = keyboardDriver
        this.gamepadDriver = gamepadDriver
        this.pointerController = new PointerInputController(
            () => this.runtime.mouseSensitivity,
            () => this.runtime.mouseKeyboard,
        )
        this.rumbleService = new RumbleService(this.nativeBridge, this.emitter)
    }

    updateRuntime(runtime: Partial<InputRuntimeConfig>): void {
        const previousPollingRate = this.runtime.pollingRate
        this.runtime = { ...this.runtime, ...runtime }
        if (runtime.pollingRate && runtime.pollingRate !== previousPollingRate && this.currentInputTransport && this.currentControlTransport) {
            this.start(this.currentInputTransport, this.currentControlTransport)
        }
    }

    setKeyboardInputEnabled(enabled: boolean): void {
        this.keyboardInputEnabled = enabled
        this.keyboardDriver.setEnabled(enabled)
        if (!enabled) {
            this.keyboardFrames = []
        }
    }

    start(inputTransport: InputTransport, controlTransport: ControlTransport): void {
        this.currentInputTransport = inputTransport
        this.currentControlTransport = controlTransport
        this.stop()
        this.debugInputPacketCount = 0
        const metadataPacket = new InputPacketEncoder(this.inputSequenceNum)
        metadataPacket.setMetadata(2)
        console.info('[player][input-service] send metadata packet')
        inputTransport.send(metadataPacket.toBuffer())
        this.keyboardDriver.start()
        this.gamepadDriver.start()
        this.gamepadDriver.run?.()
        this.inputInterval = window.setInterval(() => {
            if (
                this.keyboardInputEnabled &&
                this.runtime.legacyKeyboard &&
                this.gamepadFrames.length === 0 &&
                !this.isVirtualButtonPressing
            ) {
                const frame = this.mergeState(this.gamepadDriver.requestStates()[0] ?? DEFAULT_INPUT_FRAME(), this.keyboardDriver.requestState())
                this.queueGamepadState(frame)
                this.rumbleService.applyForcedTriggerRumble(frame, this.runtime)
            }
            if (this.runtime.touch) {
                this.pointerFrames.push(...this.pointerController.flushPointerFrames())
            }
            const metadataQueue = this.frameMetadataQueue.splice(0, 29)
            const gamepadQueue = this.gamepadFrames.splice(0, 29)
            const pointerQueue = this.pointerFrames.splice(0, 1)
            const mouseQueue = this.mouseFrames.splice(0, 29)
            const keyboardQueue = this.keyboardFrames.splice(0, 1)
            if (
                metadataQueue.length === 0 &&
        gamepadQueue.length === 0 &&
        pointerQueue.length === 0 &&
        mouseQueue.length === 0 &&
        keyboardQueue.length === 0
            ) {
                return
            }
            this.inputSequenceNum++
            const packet = new InputPacketEncoder(this.inputSequenceNum)
            packet.setData(metadataQueue, gamepadQueue, pointerQueue, mouseQueue, keyboardQueue)
            if (inputTransport.getReadyState() === 'open') {
                const buffer = packet.toBuffer()
                if (this.debugInputPacketCount < 5) {
                    console.info('[player][input-service] send packet', {
                        metadataFrames: metadataQueue.length,
                        gamepadFrames: gamepadQueue.length,
                        pointerFrames: pointerQueue.length,
                        mouseFrames: mouseQueue.length,
                        keyboardFrames: keyboardQueue.length,
                    })
                    this.debugInputPacketCount += 1
                }
                inputTransport.send(buffer)
                this.emitter.emit('stats.inputPacket', {
                    packetBytes: buffer.byteLength,
                    metadataFrames: metadataQueue.length,
                    gamepadFrames: gamepadQueue.length,
                    pointerFrames: pointerQueue.length,
                    mouseFrames: mouseQueue.length,
                    keyboardFrames: keyboardQueue.length,
                })
            }
        }, 1000 / this.runtime.pollingRate)
    }

    stop(): void {
        this.keyboardDriver.stop()
        this.gamepadDriver.stop()
        if (this.inputInterval) {
            window.clearInterval(this.inputInterval)
            this.inputInterval = undefined
        }
        this.rumbleService.destroy()
    }

    handleRumble(event: MessageEvent<ArrayBuffer>): void {
        this.rumbleService.handlePacket(event, this.runtime)
    }

    queueGamepadState(frame: InputFrame): void {
        this.gamepadFrames.push(frame)
    }

    setGamepadState(frame: InputFrame): void {
        this.gamepadDriver.setGamepadState?.(frame)
    }

    pressButtonStart(button: keyof InputFrame): void {
        this.isVirtualButtonPressing = true
        this.gamepadDriver.pressButtonStart?.(button)
    }

    pressButtonEnd(button: keyof InputFrame): void {
        this.gamepadDriver.pressButtonEnd?.(button)
        this.isVirtualButtonPressing = false
    }

    moveLeftStick(x: number, y: number): void {
        this.gamepadDriver.moveLeftStick?.(x, y)
    }

    moveRightStick(x: number, y: number): void {
        this.gamepadDriver.moveRightStick?.(x, y)
    }

    addProcessedFrame(frame: ProcessedVideoFrameMetadata): void {
        frame.frameRenderedTimeMs = performance.now()
        this.frameMetadataQueue.push(frame)
        this.emitter.emit('stats.videoFrameProcessed', frame)
    }

    onPointerMove(e: PointerEvent): void {
        this.pointerController.onPointerMove(e, (frame) => this.mouseFrames.push(frame))
    }

    onPointerDownOrUp(e: PointerEvent): void {
        this.pointerController.onPointerDownOrUp(e, (frame) => this.mouseFrames.push(frame))
    }

    onWheel(e: WheelEvent): void {
        this.pointerController.onWheel(e)
    }

    onKeyboardPointerLockedDown(event: KeyboardEvent): void {
        if (!this.keyboardInputEnabled) {
            return
        }
        this.pointerController.onKeyboardPointerLockedDown(event, (frame) => this.keyboardFrames.push(frame))
    }

    onKeyboardPointerLockedUp(event: KeyboardEvent): void {
        if (!this.keyboardInputEnabled) {
            return
        }
        this.pointerController.onKeyboardPointerLockedUp(event, (frame) => this.keyboardFrames.push(frame))
    }

    private mergeState(gpState: InputFrame, kbState: InputFrame): InputFrame {
        const merged = DEFAULT_INPUT_FRAME()
        for (const key of Object.keys(merged) as Array<keyof InputFrame>) {
            const left = gpState[key] as number
            const right = kbState[key] as number
            if (String(key).includes('XAxis') || String(key).includes('YAxis')) {
                (merged[key] as any) = Math.abs(left) > Math.abs(right) ? left : right
            } else {
                (merged[key] as any) = Math.max(left, right)
            }
        }
        return merged
    }
}
