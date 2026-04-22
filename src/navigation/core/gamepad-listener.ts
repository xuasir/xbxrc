import { events } from '../../services/events'
import { rpc } from '../../services/rpc'
import { setLastActivePadId } from './haptics'
import { inputDispatcher, NavigationIntent } from './input'

const STICK_DEADZONE = 0.5
const BUTTON_REPEAT_DELAY = 400
const BUTTON_REPEAT_RATE = 100

interface GamepadState {
  lastPressed: Record<string, number>
  repeating: Record<string, boolean>
  comboPressed: Record<string, boolean>
}

class GamepadUIListener {
  private dispose: (() => void) | null = null
  private state: Record<string, GamepadState> = {}
  private inputPolicy: 'shared' | 'ui-only' | 'stream-only' = 'shared'

  start() {
    if (this.dispose)
      return

    void rpc.gamepad.getRuntimeSnapshot().then((snapshot) => {
      this.inputPolicy = snapshot.inputPolicy
    }).catch(() => {})

    const disposeRuntime = events.on('gamepad.runtimeSnapshot', (snapshot) => {
      this.inputPolicy = snapshot.inputPolicy
    })

    this.dispose = events.on('gamepad.slotSnapshot', (snapshot) => {
      if (this.inputPolicy === 'stream-only') {
        return
      }

      const slotId = snapshot.slot
      setLastActivePadId(slotId)
      if (!this.state[slotId]) {
        this.state[slotId] = {
          lastPressed: {},
          repeating: {},
          comboPressed: {},
        }
      }

      const state = this.state[slotId]
      const buttons = snapshot.state.buttons
      const now = Date.now()
      this.checkCombo(state, 'menu-view-toggle', buttons.menu > 0.5 && buttons.view > 0.5)

      // Map LogicalButtonsStateDto to NavigationIntent
      this.checkButton(now, state, 'south', buttons.south > 0.5, NavigationIntent.Action)
      this.checkButton(now, state, 'east', buttons.east > 0.5, NavigationIntent.Back)
      // LB/RB → 一级页面导航
      this.checkButton(now, state, 'l1', buttons.l1 > 0.5, NavigationIntent.PagePrev)
      this.checkButton(now, state, 'r1', buttons.r1 > 0.5, NavigationIntent.PageNext)
      // LT/RT → 二级 Tab/区域导航
      this.checkButton(now, state, 'l2', buttons.l2 > 0.5, NavigationIntent.TabPrev)
      this.checkButton(now, state, 'r2', buttons.r2 > 0.5, NavigationIntent.TabNext)
      this.checkButton(now, state, 'view', buttons.view > 0.5, NavigationIntent.View)
      this.checkButton(now, state, 'menu', buttons.menu > 0.5, NavigationIntent.Menu)
      this.checkButton(now, state, 'dpadUp', buttons.dpadUp > 0.5, NavigationIntent.Up)
      this.checkButton(now, state, 'dpadDown', buttons.dpadDown > 0.5, NavigationIntent.Down)
      this.checkButton(now, state, 'dpadLeft', buttons.dpadLeft > 0.5, NavigationIntent.Left)
      this.checkButton(now, state, 'dpadRight', buttons.dpadRight > 0.5, NavigationIntent.Right)

      // Sticks
      const leftStick = snapshot.state.leftStick
      this.checkAxis(now, state, 'ls-left', leftStick.x < -STICK_DEADZONE, NavigationIntent.Left)
      this.checkAxis(now, state, 'ls-right', leftStick.x > STICK_DEADZONE, NavigationIntent.Right)
      this.checkAxis(now, state, 'ls-up', leftStick.y < -STICK_DEADZONE, NavigationIntent.Up)
      this.checkAxis(now, state, 'ls-down', leftStick.y > STICK_DEADZONE, NavigationIntent.Down)
    })

    const disposePad = this.dispose
    this.dispose = () => {
      disposeRuntime()
      disposePad?.()
    }
  }

  stop() {
    if (this.dispose) {
      this.dispose()
      this.dispose = null
    }
  }

  private checkButton(now: number, state: GamepadState, key: string, isPressed: boolean, intent: NavigationIntent) {
    this.handleInputState(now, state, key, isPressed, intent)
  }

  private checkAxis(now: number, state: GamepadState, key: string, isPressed: boolean, intent: NavigationIntent) {
    this.handleInputState(now, state, key, isPressed, intent)
  }

  private handleInputState(now: number, state: GamepadState, key: string, isPressed: boolean, intent: NavigationIntent) {
    if (isPressed) {
      const lastPressedTime = state.lastPressed[key]
      if (!lastPressedTime) {
        state.lastPressed[key] = now
        inputDispatcher.dispatch(intent)
      }
      else {
        const isRepeating = state.repeating[key]
        const delay = isRepeating ? BUTTON_REPEAT_RATE : BUTTON_REPEAT_DELAY
        if (now - lastPressedTime > delay) {
          state.lastPressed[key] = now
          state.repeating[key] = true
          inputDispatcher.dispatch(intent)
        }
      }
    }
    else {
      if (state.lastPressed[key]) {
        delete state.lastPressed[key]
        delete state.repeating[key]
      }
    }
  }

  private checkCombo(state: GamepadState, key: string, active: boolean): void {
    if (active) {
      if (state.comboPressed[key] === true) {
        return
      }
      state.comboPressed[key] = true
      window.dispatchEvent(
        new CustomEvent('stream-menu-toggle-requested', {
          detail: { source: 'gamepad', combo: 'menu+view' },
        }),
      )
      return
    }

    if (state.comboPressed[key] === true) {
      delete state.comboPressed[key]
    }
  }
}

export const gamepadUIListener = new GamepadUIListener()
