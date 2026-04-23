import { events } from '../../services/events'
import { rpc } from '../../services/rpc'
import { setLastActivePadId } from './haptics'
import { inputDispatcher, NavigationIntent } from './input'

const STICK_DEADZONE = 0.5
// UI 导航长按：默认稍慢一点，减少长列表的“连跳失控感”
const BUTTON_REPEAT_DELAY = 450
const BUTTON_REPEAT_RATE = 140

interface GamepadState {
  pressed: Record<string, boolean>
  repeatTimers: Record<string, number | undefined>
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
      this.updateInputPolicy(snapshot.inputPolicy)
    }).catch(() => {})

    const disposeRuntime = events.on('gamepad.runtimeSnapshot', (snapshot) => {
      this.updateInputPolicy(snapshot.inputPolicy)
    })

    this.dispose = events.on('gamepad.slotSnapshot', (snapshot) => {
      if (this.inputPolicy === 'stream-only') {
        return
      }

      const slotId = snapshot.slot
      setLastActivePadId(slotId)
      if (!this.state[slotId]) {
        this.state[slotId] = {
          pressed: {},
          repeatTimers: {},
          comboPressed: {},
        }
      }

      const state = this.state[slotId]
      const buttons = snapshot.state.buttons
      this.checkCombo(state, 'menu-view-toggle', buttons.menu > 0.5 && buttons.view > 0.5)

      // Map LogicalButtonsStateDto to NavigationIntent
      this.checkButton(state, 'south', buttons.south > 0.5, NavigationIntent.Action)
      this.checkButton(state, 'east', buttons.east > 0.5, NavigationIntent.Back)
      // LB/RB → 一级页面导航
      this.checkButton(state, 'l1', buttons.l1 > 0.5, NavigationIntent.PagePrev)
      this.checkButton(state, 'r1', buttons.r1 > 0.5, NavigationIntent.PageNext)
      // LT/RT → 二级 Tab/区域导航
      this.checkButton(state, 'l2', buttons.l2 > 0.5, NavigationIntent.TabPrev)
      this.checkButton(state, 'r2', buttons.r2 > 0.5, NavigationIntent.TabNext)
      this.checkButton(state, 'view', buttons.view > 0.5, NavigationIntent.View)
      this.checkButton(state, 'menu', buttons.menu > 0.5, NavigationIntent.Menu)
      this.checkButton(state, 'dpadUp', buttons.dpadUp > 0.5, NavigationIntent.Up)
      this.checkButton(state, 'dpadDown', buttons.dpadDown > 0.5, NavigationIntent.Down)
      this.checkButton(state, 'dpadLeft', buttons.dpadLeft > 0.5, NavigationIntent.Left)
      this.checkButton(state, 'dpadRight', buttons.dpadRight > 0.5, NavigationIntent.Right)

      // Sticks
      const leftStick = snapshot.state.leftStick
      this.checkAxis(state, 'ls-left', leftStick.x < -STICK_DEADZONE, NavigationIntent.Left)
      this.checkAxis(state, 'ls-right', leftStick.x > STICK_DEADZONE, NavigationIntent.Right)
      this.checkAxis(state, 'ls-up', leftStick.y < -STICK_DEADZONE, NavigationIntent.Up)
      this.checkAxis(state, 'ls-down', leftStick.y > STICK_DEADZONE, NavigationIntent.Down)
    })

    const disposePad = this.dispose
    this.dispose = () => {
      disposeRuntime()
      disposePad?.()
    }
  }

  stop() {
    this.resetAllInputState()
    if (this.dispose) {
      this.dispose()
      this.dispose = null
    }
  }

  private checkButton(state: GamepadState, key: string, isPressed: boolean, intent: NavigationIntent) {
    this.handleInputState(state, key, isPressed, intent)
  }

  private checkAxis(state: GamepadState, key: string, isPressed: boolean, intent: NavigationIntent) {
    this.handleInputState(state, key, isPressed, intent)
  }

  private handleInputState(state: GamepadState, key: string, isPressed: boolean, intent: NavigationIntent) {
    if (isPressed) {
      if (!state.pressed[key]) {
        state.pressed[key] = true
        inputDispatcher.dispatch(intent)
        this.startRepeatTimer(state, key, intent)
      }
    }
    else {
      this.clearInputState(state, key)
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

  private updateInputPolicy(nextPolicy: 'shared' | 'ui-only' | 'stream-only') {
    if (this.inputPolicy !== nextPolicy) {
      // 策略切换时统一清理，避免残留重复触发或组合键卡住。
      this.resetAllInputState()
    }
    this.inputPolicy = nextPolicy
  }

  private startRepeatTimer(state: GamepadState, key: string, intent: NavigationIntent) {
    this.clearRepeatTimer(state, key)

    // Repeat 改为独立定时器，不再依赖 slotSnapshot 的到达节奏。
    state.repeatTimers[key] = window.setTimeout(() => {
      if (!state.pressed[key]) {
        this.clearRepeatTimer(state, key)
        return
      }

      inputDispatcher.dispatch(intent)
      state.repeatTimers[key] = window.setInterval(() => {
        if (!state.pressed[key]) {
          this.clearRepeatTimer(state, key)
          return
        }
        inputDispatcher.dispatch(intent)
      }, BUTTON_REPEAT_RATE)
    }, BUTTON_REPEAT_DELAY)
  }

  private clearRepeatTimer(state: GamepadState, key: string) {
    const timer = state.repeatTimers[key]
    if (timer !== undefined) {
      window.clearTimeout(timer)
      delete state.repeatTimers[key]
    }
  }

  private clearInputState(state: GamepadState, key: string) {
    if (state.pressed[key]) {
      delete state.pressed[key]
    }
    this.clearRepeatTimer(state, key)
  }

  private resetPadState(state: GamepadState) {
    for (const key of Object.keys(state.pressed)) {
      this.clearInputState(state, key)
    }

    for (const key of Object.keys(state.repeatTimers)) {
      this.clearRepeatTimer(state, key)
    }

    state.comboPressed = {}
  }

  private resetAllInputState() {
    for (const state of Object.values(this.state)) {
      this.resetPadState(state)
    }
    this.state = {}
  }
}

export const gamepadUIListener = new GamepadUIListener()
