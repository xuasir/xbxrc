import type { GamepadRuntimeSnapshotDto, LogicalPadSnapshotDto } from '@shared/gamepad/contract'
import { events } from '../../services/events'
import { rpc } from '../../services/rpc'
import { setLastActivePadId } from './haptics'
import { inputDispatcher, NavigationIntent } from './input'

const STICK_DEADZONE = 0.5
// UI 导航长按：默认稍慢一点，减少长列表的“连跳失控感”
const BUTTON_REPEAT_DELAY = 450
const BUTTON_REPEAT_RATE = 140
const GAMEPAD_UI_RESET_EVENT = 'xbxrc:gamepad:ui-listener-reset-requested'
let lastUiTraceSignature = ''

function recordUiTrace(event: string, payload: Record<string, unknown>): void {
  const signature = `${event}:${JSON.stringify(payload)}`
  if (signature === lastUiTraceSignature) {
    return
  }
  lastUiTraceSignature = signature
  void rpc.runtimeTrace.recordEvent({
    event,
    payload,
  }).catch(() => {})
}

interface GamepadState {
  pressed: Record<string, boolean>
  repeatTimers: Record<string, number | undefined>
  comboPressed: Record<string, boolean>
}

class GamepadUIListener {
  private dispose: (() => void) | null = null
  private state: Record<string, GamepadState> = {}
  private inputPolicy: 'shared' | 'ui-only' | 'stream-only' = 'shared'
  private handleResetRequested = () => {
    this.resetAllInputState()
  }

  private readonly handleWindowFocus = () => {
    void this.refreshRuntimeSnapshot('window-focus')
  }

  private readonly handleVisibilityChange = () => {
    if (document.visibilityState !== 'visible') {
      return
    }
    void this.refreshRuntimeSnapshot('document-visible')
  }

  start() {
    if (this.dispose)
      return

    recordUiTrace('gamepadUiListenerStarted', {})

    void this.refreshRuntimeSnapshot('listener-start')

    const disposeRuntime = events.on('gamepad.runtimeSnapshot', (snapshot) => {
      this.updateInputPolicy(snapshot.inputPolicy)
      this.applyRuntimeSnapshot(snapshot)
    })
    const disposeBaseline = events.on('gamepad.inputBaselineAbsorbed', () => {
      this.resetAllInputState()
    })
    window.addEventListener(GAMEPAD_UI_RESET_EVENT, this.handleResetRequested)
    window.addEventListener('focus', this.handleWindowFocus)
    document.addEventListener('visibilitychange', this.handleVisibilityChange)

    const disposeSlot = events.on('gamepad.slotSnapshot', (snapshot) => {
      this.applySlotSnapshot(snapshot)
    })

    this.dispose = () => {
      disposeRuntime()
      disposeBaseline()
      window.removeEventListener(GAMEPAD_UI_RESET_EVENT, this.handleResetRequested)
      window.removeEventListener('focus', this.handleWindowFocus)
      document.removeEventListener('visibilitychange', this.handleVisibilityChange)
      disposeSlot()
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
        recordUiTrace('gamepadUiIntentDispatched', {
          intent,
          key,
          source: 'press',
        })
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

      recordUiTrace('gamepadUiIntentDispatched', {
        intent,
        key,
        source: 'repeat-start',
      })
      inputDispatcher.dispatch(intent)
      state.repeatTimers[key] = window.setInterval(() => {
        if (!state.pressed[key]) {
          this.clearRepeatTimer(state, key)
          return
        }
        recordUiTrace('gamepadUiIntentDispatched', {
          intent,
          key,
          source: 'repeat-tick',
        })
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

  private async refreshRuntimeSnapshot(reason: string): Promise<void> {
    try {
      const snapshot = await rpc.gamepad.getRuntimeSnapshot()
      this.updateInputPolicy(snapshot.inputPolicy)
      this.applyRuntimeSnapshot(snapshot)
      recordUiTrace('gamepadUiRuntimeSnapshotRefreshed', {
        reason,
        inputPolicy: snapshot.inputPolicy,
        slotCount: snapshot.slots.length,
      })
    }
    catch {
      // 主动补快照失败不影响现有事件流。
    }
  }

  private applyRuntimeSnapshot(snapshot: GamepadRuntimeSnapshotDto): void {
    recordUiTrace('gamepadUiRuntimeSnapshotApplied', {
      inputPolicy: snapshot.inputPolicy,
      slotCount: snapshot.slots.length,
      maxSampleSeq: snapshot.slots.reduce((max, slot) => Math.max(max, slot.sampleSeq), 0),
    })
    for (const slot of snapshot.slots) {
      this.applySlotSnapshot(slot)
    }
  }

  private applySlotSnapshot(snapshot: LogicalPadSnapshotDto): void {
    if (this.inputPolicy === 'stream-only') {
      recordUiTrace('gamepadUiSlotIgnored', {
        reason: 'stream-only',
        slot: snapshot.slot,
        sampleSeq: snapshot.sampleSeq,
      })
      return
    }

    recordUiTrace('gamepadUiSlotApplied', {
      slot: snapshot.slot,
      sampleSeq: snapshot.sampleSeq,
      sampledAtMs: snapshot.sampledAtMs,
      south: snapshot.state.buttons.south,
      east: snapshot.state.buttons.east,
      menu: snapshot.state.buttons.menu,
      view: snapshot.state.buttons.view,
      dpadUp: snapshot.state.buttons.dpadUp,
      dpadDown: snapshot.state.buttons.dpadDown,
    })

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

    this.checkButton(state, 'south', buttons.south > 0.5, NavigationIntent.Action)
    this.checkButton(state, 'east', buttons.east > 0.5, NavigationIntent.Back)
    this.checkButton(state, 'l1', buttons.l1 > 0.5, NavigationIntent.PagePrev)
    this.checkButton(state, 'r1', buttons.r1 > 0.5, NavigationIntent.PageNext)
    this.checkButton(state, 'l2', buttons.l2 > 0.5, NavigationIntent.TabPrev)
    this.checkButton(state, 'r2', buttons.r2 > 0.5, NavigationIntent.TabNext)
    this.checkButton(state, 'view', buttons.view > 0.5, NavigationIntent.View)
    this.checkButton(state, 'menu', buttons.menu > 0.5, NavigationIntent.Menu)
    this.checkButton(state, 'dpadUp', buttons.dpadUp > 0.5, NavigationIntent.Up)
    this.checkButton(state, 'dpadDown', buttons.dpadDown > 0.5, NavigationIntent.Down)
    this.checkButton(state, 'dpadLeft', buttons.dpadLeft > 0.5, NavigationIntent.Left)
    this.checkButton(state, 'dpadRight', buttons.dpadRight > 0.5, NavigationIntent.Right)

    const leftStick = snapshot.state.leftStick
    this.checkAxis(state, 'ls-left', leftStick.x < -STICK_DEADZONE, NavigationIntent.Left)
    this.checkAxis(state, 'ls-right', leftStick.x > STICK_DEADZONE, NavigationIntent.Right)
    this.checkAxis(state, 'ls-up', leftStick.y < -STICK_DEADZONE, NavigationIntent.Up)
    this.checkAxis(state, 'ls-down', leftStick.y > STICK_DEADZONE, NavigationIntent.Down)
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

export function requestGamepadUiListenerReset(reason: string): void {
  window.dispatchEvent(new CustomEvent(GAMEPAD_UI_RESET_EVENT, {
    detail: { reason },
  }))
}
