import type { InputSnapshot } from '@shared/gamepad/input-controller'
import { inputController } from '@shared/gamepad/runtime'
import { inputDispatcher, NavigationIntent } from './input'

const REPEAT_DELAY = 450
const REPEAT_RATE = 140
const ONCE = new Set([NavigationIntent.Action, NavigationIntent.Back, NavigationIntent.View, NavigationIntent.Menu])

class GamepadUIListener {
  private dispose: (() => void) | undefined
  private pressed = new Set<NavigationIntent>()
  private timers = new Map<NavigationIntent, ReturnType<typeof setTimeout>>()

  start(): void {
    this.dispose ??= inputController.subscribe(snapshot => this.apply(snapshot))
  }

  stop(): void {
    this.dispose?.()
    this.dispose = undefined
    this.reset()
  }

  reset(): void {
    this.pressed.clear()
    for (const timer of this.timers.values()) clearTimeout(timer)
    this.timers.clear()
  }

  private apply(snapshot: InputSnapshot): void {
    const { buttons: b, leftStick: stick } = snapshot.uiState
    if (snapshot.owner !== 'ui' || snapshot.exclusiveCapture) {
      this.reset()
      return
    }
    const intents: Array<[NavigationIntent, boolean]> = [
      [NavigationIntent.Action, b.south > 0.5],
      [NavigationIntent.Back, b.east > 0.5],
      [NavigationIntent.PagePrev, b.l1 > 0.5],
      [NavigationIntent.PageNext, b.r1 > 0.5],
      [NavigationIntent.TabPrev, b.l2 > 0.5],
      [NavigationIntent.TabNext, b.r2 > 0.5],
      [NavigationIntent.View, b.view > 0.5],
      [NavigationIntent.Menu, b.menu > 0.5],
      [NavigationIntent.Up, b.dpadUp > 0.5 || stick.y < -0.5],
      [NavigationIntent.Down, b.dpadDown > 0.5 || stick.y > 0.5],
      [NavigationIntent.Left, b.dpadLeft > 0.5 || stick.x < -0.5],
      [NavigationIntent.Right, b.dpadRight > 0.5 || stick.x > 0.5],
    ]
    for (const [intent, active] of intents) {
      if (inputController.getSnapshot() !== snapshot)
        return
      if (active && !this.pressed.has(intent)) {
        this.pressed.add(intent)
        inputDispatcher.dispatch(intent)
        if (inputController.getSnapshot() !== snapshot)
          return
        if (!ONCE.has(intent))
          this.repeat(intent, REPEAT_DELAY)
      }
      else if (!active) {
        this.pressed.delete(intent)
        clearTimeout(this.timers.get(intent))
        this.timers.delete(intent)
      }
    }
  }

  private repeat(intent: NavigationIntent, delay: number): void {
    this.timers.set(intent, setTimeout(() => {
      const snapshot = inputController.getSnapshot()
      if (!this.pressed.has(intent) || snapshot.owner !== 'ui' || snapshot.exclusiveCapture)
        return
      inputDispatcher.dispatch(intent)
      if (this.pressed.has(intent) && inputController.getSnapshot() === snapshot)
        this.repeat(intent, REPEAT_RATE)
    }, delay))
  }
}

export const gamepadUIListener = new GamepadUIListener()
