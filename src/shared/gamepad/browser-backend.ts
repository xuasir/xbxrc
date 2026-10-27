import type { GamepadBackend, GamepadSnapshot, RumbleResult } from './backend'
import type { GamepadRumbleEffectDto, LogicalPadStateDto } from './contract'
import type { ButtonKey } from './state'
import { axisValue, buttonValue, neutralState } from './state'

interface BrowserActuator {
  effects?: readonly string[]
  type?: string
  playEffect?: (type: 'dual-rumble', parameters: {
    duration: number
    startDelay: number
    strongMagnitude: number
    weakMagnitude: number
  }) => Promise<string>
  reset?: () => Promise<string>
}

export interface BrowserPad {
  index: number
  id: string
  connected: boolean
  mapping: string
  buttons: readonly { value: number, pressed: boolean }[]
  axes: readonly number[]
  vibrationActuator?: BrowserActuator | null
}

export interface BrowserInputEnvironment {
  getGamepads: () => ArrayLike<BrowserPad | null>
  requestFrame: (callback: () => void) => number
  cancelFrame: (id: number) => void
  now: () => number
}

const STANDARD_BUTTONS: readonly ButtonKey[] = [
  'south',
  'east',
  'west',
  'north',
  'l1',
  'r1',
  'l2',
  'r2',
  'view',
  'menu',
  'l3',
  'r3',
  'dpadUp',
  'dpadDown',
  'dpadLeft',
  'dpadRight',
  'home',
]

export function browserPadState(pad: BrowserPad): LogicalPadStateDto {
  const state = neutralState()
  if (pad.mapping !== 'standard')
    return state
  STANDARD_BUTTONS.forEach((key, index) => {
    state.buttons[key] = buttonValue(pad.buttons[index]?.value)
  })
  state.leftStick = { x: axisValue(pad.axes[0]), y: axisValue(pad.axes[1]) }
  state.rightStick = { x: axisValue(pad.axes[2]), y: axisValue(pad.axes[3]) }
  state.leftTrigger = state.buttons.l2
  state.rightTrigger = state.buttons.r2
  return state
}

function supportsRumble(pad: BrowserPad): boolean {
  const actuator = pad.vibrationActuator
  return typeof actuator?.playEffect === 'function'
    && (actuator.effects?.includes('dual-rumble') === true || actuator.type === 'dual-rumble')
}

export class BrowserGamepadBackend implements GamepadBackend {
  readonly kind = 'browser' as const
  private listener: ((snapshot: GamepadSnapshot) => void) | undefined
  private frame: number | undefined
  private sequence = 0
  private deviceGeneration = 0
  private generation = 0
  private devices = new Map<number, { id: string, name: string, pad: BrowserPad }>()

  constructor(private readonly environment: BrowserInputEnvironment = {
    getGamepads: () => navigator.getGamepads(),
    requestFrame: callback => requestAnimationFrame(callback),
    cancelFrame: id => cancelAnimationFrame(id),
    now: () => performance.now(),
  }) {}

  async start(onSnapshot: (snapshot: GamepadSnapshot) => void): Promise<void> {
    if (this.listener)
      return
    this.listener = onSnapshot
    this.sequence = 0
    const generation = ++this.generation
    const tick = () => {
      if (generation !== this.generation || !this.listener)
        return
      this.frame = undefined
      try {
        this.sample()
      }
      finally {
        if (generation === this.generation)
          this.frame = this.environment.requestFrame(tick)
      }
    }
    tick()
  }

  async stop(): Promise<void> {
    ++this.generation
    this.listener = undefined
    if (this.frame !== undefined)
      this.environment.cancelFrame(this.frame)
    this.frame = undefined
    const stopping = [...this.devices.values()].map(device => this.stopRumble(device.id))
    this.devices.clear()
    await Promise.all(stopping)
  }

  private sample(): void {
    const snapshot: GamepadSnapshot = {
      devices: [],
      samples: [],
      sequence: ++this.sequence,
      observedAt: this.environment.now(),
      error: null,
    }
    try {
      const pads = this.environment.getGamepads()
      const next = new Map<number, { id: string, name: string, pad: BrowserPad }>()
      for (let index = 0; index < pads.length; index++) {
        const pad = pads[index]
        if (!pad?.connected)
          continue
        const previous = this.devices.get(pad.index)
        const id = previous?.name === pad.id
          ? previous.id
          : `browser:${pad.index}:${++this.deviceGeneration}`
        next.set(pad.index, { id, name: pad.id, pad })
        snapshot.devices.push({ id, name: pad.id, standard: pad.mapping === 'standard', rumble: supportsRumble(pad) })
        snapshot.samples.push({ deviceId: id, state: browserPadState(pad) })
      }
      for (const device of this.devices.values()) {
        if (![...next.values()].some(current => current.id === device.id))
          void this.stopRumble(device.id)
      }
      this.devices = next
    }
    catch (error) {
      snapshot.error = error instanceof Error ? error.message : String(error)
      for (const device of this.devices.values())
        void this.stopRumble(device.id)
      this.devices.clear()
    }
    this.listener?.(snapshot)
  }

  async rumble(deviceId: string, effect: GamepadRumbleEffectDto): Promise<RumbleResult> {
    const device = [...this.devices.values()].find(device => device.id === deviceId)
    if (!device || !supportsRumble(device.pad))
      return 'unsupported'
    const generation = this.generation
    try {
      const result = await device.pad.vibrationActuator!.playEffect!('dual-rumble', {
        startDelay: Number.isFinite(effect.startDelayMs) ? Math.max(0, Math.min(1000, effect.startDelayMs)) : 0,
        duration: Number.isFinite(effect.durationMs) ? Math.max(0, Math.min(1000, effect.durationMs)) : 0,
        strongMagnitude: buttonValue(effect.strongMagnitude),
        weakMagnitude: buttonValue(effect.weakMagnitude),
      })
      return generation === this.generation && this.devices.get(device.pad.index)?.id === deviceId
        && result === 'complete'
        ? 'applied'
        : 'failed'
    }
    catch {
      return 'failed'
    }
  }

  async stopRumble(deviceId: string): Promise<void> {
    const actuator = [...this.devices.values()].find(device => device.id === deviceId)?.pad.vibrationActuator
    try {
      if (actuator?.reset)
        await actuator.reset()
      else if (actuator?.playEffect)
        await actuator.playEffect('dual-rumble', { startDelay: 0, duration: 0, strongMagnitude: 0, weakMagnitude: 0 })
    }
    catch {
      // 设备断开时停止震动可失败，输入生命周期仍继续关闭。
    }
  }
}
