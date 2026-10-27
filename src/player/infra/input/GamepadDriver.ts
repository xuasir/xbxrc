import type { LogicalButtonDto } from '@shared/gamepad/contract'
import type { GamepadFrame } from '../../domain/input'
import { inputController } from '@shared/gamepad/runtime'
import { clonePadState, neutralState, stateEquals } from '@shared/gamepad/state'

export interface GamepadDriverDelegate {
  onGamepadAdded: (index: number) => void
  onGamepadRemoved: (index: number) => void
  onFrame: (frame: GamepadFrame) => void
}

export class GamepadDriver {
  private dispose: (() => void) | undefined
  private virtual = neutralState()
  private lastSent = neutralState()
  private lastSentAt = 0

  constructor(private readonly delegate: GamepadDriverDelegate) {}

  start(): void {
    if (this.dispose)
      return
    this.delegate.onGamepadAdded(0)
    this.lastSentAt = Number.NEGATIVE_INFINITY
    this.dispose = inputController.subscribe((snapshot) => {
      const now = performance.now()
      if (stateEquals(this.lastSent, snapshot.gameState) && now - this.lastSentAt < 100)
        return
      this.lastSent = clonePadState(snapshot.gameState)
      this.lastSentAt = now
      this.delegate.onFrame({ gamepadIndex: 0, state: clonePadState(snapshot.gameState) })
    })
  }

  stop(): void {
    if (!this.dispose)
      return
    this.dispose?.()
    this.dispose = undefined
    this.virtual = neutralState()
    inputController.setVirtualState(this.virtual)
    inputController.setSystemHome(false)
    this.delegate.onGamepadRemoved(0)
  }

  requestStates(): GamepadFrame[] {
    return [{ gamepadIndex: 0, state: clonePadState(inputController.getSnapshot().gameState) }]
  }

  setGamepadState(frame: GamepadFrame): void {
    this.virtual = clonePadState(frame.state)
    inputController.setVirtualState(this.virtual)
  }

  pressButtonStart(button: LogicalButtonDto): void { this.setButton(button, 1) }
  pressButtonEnd(button: LogicalButtonDto): void { this.setButton(button, 0) }

  moveLeftStick(x: number, y: number): void {
    this.virtual.leftStick = { x, y }
    inputController.setVirtualState(this.virtual)
  }

  moveRightStick(x: number, y: number): void {
    this.virtual.rightStick = { x, y }
    inputController.setVirtualState(this.virtual)
  }

  private setButton(button: LogicalButtonDto, value: number): void {
    if (button === 'home') {
      inputController.setSystemHome(value > 0)
      return
    }
    const key = button.replace(/-([a-z])/g, (_, letter: string) => letter.toUpperCase()) as keyof typeof this.virtual.buttons
    this.virtual.buttons[key] = value
    if (key === 'l2')
      this.virtual.leftTrigger = value
    if (key === 'r2')
      this.virtual.rightTrigger = value
    inputController.setVirtualState(this.virtual)
  }
}
