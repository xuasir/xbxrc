import type { GamepadRuntimeSnapshotDto, LogicalPadSnapshotDto } from '@shared/gamepad/contract'
import {
  businessInputArbiter,
  type BusinessInputTracePayload,
  toBusinessInputTracePayload,
} from '@shared/gamepad/business-input-arbiter'
import { events } from '../../services/events'
import { rpc } from '../../services/rpc'
import { setLastActivePadId } from './haptics'
import { inputDispatcher, NavigationIntent } from './input'

const STICK_DEADZONE = 0.5
// UI 导航长按：默认稍慢一点，减少长列表的“连跳失控感”
const BUTTON_REPEAT_DELAY = 450
const BUTTON_REPEAT_RATE = 140
const GAMEPAD_UI_RESET_EVENT = 'xbxrc:gamepad:ui-listener-reset-requested'

function shouldRepeatGamepadUiIntent(intent: NavigationIntent): boolean {
  switch (intent) {
    case NavigationIntent.Action:
    case NavigationIntent.Back:
    case NavigationIntent.View:
    case NavigationIntent.Menu:
      return false
    default:
      return true
  }
}

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

function getBusinessInputTracePayload(): BusinessInputTracePayload {
  return toBusinessInputTracePayload({
    state: businessInputArbiter.getState(),
    owner: businessInputArbiter.getOwner(),
  })
}

interface GamepadState {
  pressed: Record<string, boolean>
  repeatTimers: Record<string, number | undefined>
  comboPressed: Record<string, boolean>
}

class GamepadUIListener {
  private dispose: (() => void) | null = null
  private state: Record<string, GamepadState> = {}
  private handleResetRequested = () => {
    this.resetAllInputState()
  }

  start() {
    if (this.dispose)
      return

    recordUiTrace('gamepadUiListenerStarted', {})

    void this.refreshRuntimeSnapshot('listener-start')

    const disposeRuntime = events.on('gamepad.runtimeSnapshot', (snapshot) => {
      this.applyRuntimeSnapshot(snapshot)
    })
    const disposeGate = events.on('gamepad.inputGateChanged', () => {
      this.resetAllInputState()
    })
    window.addEventListener(GAMEPAD_UI_RESET_EVENT, this.handleResetRequested)

    const disposeSlot = events.on('gamepad.slotSnapshot', (snapshot) => {
      this.applySlotSnapshot(snapshot)
    })

    this.dispose = () => {
      disposeRuntime()
      disposeGate()
      window.removeEventListener(GAMEPAD_UI_RESET_EVENT, this.handleResetRequested)
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
        // 确认/返回/系统键不应长按连发：否则会连续 `.click()` 或对已卸载的菜单重复派发，
        // 表现为菜单不随 A 关闭、回到串流后马达/UI 震动停不下来的现象。
        if (shouldRepeatGamepadUiIntent(intent)) {
          this.startRepeatTimer(state, key, intent)
        }
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
      this.applyRuntimeSnapshot(snapshot)
      recordUiTrace('gamepadUiRuntimeSnapshotRefreshed', {
        reason,
        streamPadForwarding: snapshot.streamPadForwarding ?? false,
        inputGate: snapshot.inputGate ?? 'open',
        slotCount: snapshot.slots.length,
        ...getBusinessInputTracePayload(),
      })
    }
    catch {
      // 主动补快照失败不影响现有事件流。
    }
  }

  private applyRuntimeSnapshot(snapshot: GamepadRuntimeSnapshotDto): void {
    recordUiTrace('gamepadUiRuntimeSnapshotApplied', {
      streamPadForwarding: snapshot.streamPadForwarding ?? false,
      inputGate: snapshot.inputGate ?? 'open',
      slotCount: snapshot.slots.length,
      maxSampleSeq: snapshot.slots.reduce((max, slot) => Math.max(max, slot.sampleSeq), 0),
      ...getBusinessInputTracePayload(),
    })
    if (snapshot.inputGate !== 'open') {
      return
    }
    for (const slot of snapshot.slots) {
      this.applySlotSnapshot(slot)
    }
  }

  private applySlotSnapshot(snapshot: LogicalPadSnapshotDto): void {
    if (businessInputArbiter.getOwner() !== 'ui') {
      recordUiTrace('gamepadUiSlotIgnored', {
        reason: 'business-input-owner-not-ui',
        slot: snapshot.slot,
        sampleSeq: snapshot.sampleSeq,
        ...getBusinessInputTracePayload(),
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
      ...getBusinessInputTracePayload(),
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
