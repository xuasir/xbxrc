import type { DeviceSample, GamepadBackend, GamepadSnapshot, InputDevice, InputSamplingObservation, RumbleResult } from './backend'
import type { GamepadRumbleEffectDto, LogicalPadStateDto } from './contract'
import type { ButtonKey } from './state'
import { LatencyWindow } from '../latency-window'
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
  startTimer?: (callback: () => void, intervalMs: number) => ReturnType<typeof setInterval>
  stopTimer?: (id: ReturnType<typeof setInterval>) => void
}

const SAMPLE_INTERVAL_MS = 4
const SNAPSHOT_HEARTBEAT_MS = 100

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

function padMatches(pad: BrowserPad, state: LogicalPadStateDto): boolean {
  if (pad.mapping !== 'standard')
    return true
  return STANDARD_BUTTONS.every((key, index) => state.buttons[key] === buttonValue(pad.buttons[index]?.value))
    && state.leftStick.x === axisValue(pad.axes[0]) && state.leftStick.y === axisValue(pad.axes[1])
    && state.rightStick.x === axisValue(pad.axes[2]) && state.rightStick.y === axisValue(pad.axes[3])
}

interface BrowserDevice {
  id: string
  name: string
  pad: BrowserPad
  device: InputDevice
  sample: DeviceSample
}

export class BrowserGamepadBackend implements GamepadBackend {
  readonly kind = 'browser' as const
  private listener: ((snapshot: GamepadSnapshot) => void) | undefined
  private frame: number | undefined
  private timer?: ReturnType<typeof setInterval>
  private lastPublished?: GamepadSnapshot
  private sequence = 0
  private deviceGeneration = 0
  private generation = 0
  private readonly devices = new Map<number, BrowserDevice>()
  private readonly seen = new Set<number>()
  private readonly sampleGap = new LatencyWindow()
  private readonly readCost = new LatencyWindow()
  private readonly dispatchCost = new LatencyWindow()
  private lastSampleAt?: number
  private publishedCount = 0
  private statsStartedAt = 0
  private statsPollCount = 0
  private statsPublishedCount = 0
  private sampling?: InputSamplingObservation

  constructor(private readonly environment: BrowserInputEnvironment = {
    getGamepads: () => navigator.getGamepads(),
    requestFrame: callback => requestAnimationFrame(callback),
    cancelFrame: id => cancelAnimationFrame(id),
    now: () => performance.now(),
    startTimer: (callback, intervalMs) => setInterval(callback, intervalMs),
    stopTimer: id => clearInterval(id),
  }) {}

  async start(onSnapshot: (snapshot: GamepadSnapshot) => void): Promise<void> {
    if (this.listener)
      return
    this.listener = onSnapshot
    this.sequence = 0
    this.lastPublished = undefined
    this.lastSampleAt = undefined
    this.publishedCount = this.statsPollCount = this.statsPublishedCount = 0
    this.statsStartedAt = this.environment.now()
    this.sampling = undefined
    this.sampleGap.reset()
    this.readCost.reset()
    this.dispatchCost.reset()
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
    // rAF 与定时器共用同一设备源；补采缩小短按窗口，相同状态只发布心跳。
    if (generation === this.generation && this.listener) {
      this.timer = this.environment.startTimer?.(() => {
        if (generation === this.generation && this.listener)
          this.sample()
      }, SAMPLE_INTERVAL_MS)
    }
  }

  async stop(): Promise<void> {
    ++this.generation
    this.listener = undefined
    if (this.frame !== undefined)
      this.environment.cancelFrame(this.frame)
    this.frame = undefined
    if (this.timer !== undefined)
      this.environment.stopTimer?.(this.timer)
    this.timer = undefined
    const stopping = [...this.devices.values()].map(device => this.stopRumble(device.id))
    this.devices.clear()
    await Promise.all(stopping)
  }

  private sample(): void {
    const observedAt = this.environment.now()
    if (this.lastSampleAt !== undefined)
      this.sampleGap.record(observedAt - this.lastSampleAt)
    this.lastSampleAt = observedAt
    this.sequence++
    let changed = !this.lastPublished
    let error: string | null = null
    this.seen.clear()
    try {
      const pads = this.environment.getGamepads()
      for (let index = 0; index < pads.length; index++) {
        const pad = pads[index]
        if (!pad?.connected)
          continue
        this.seen.add(pad.index)
        let previous = this.devices.get(pad.index)
        if (previous?.name !== pad.id) {
          if (previous)
            void this.stopRumble(previous.id)
          previous = undefined
        }
        const standard = pad.mapping === 'standard'
        const rumble = supportsRumble(pad)
        if (!previous) {
          const id = `browser:${pad.index}:${++this.deviceGeneration}`
          this.devices.set(pad.index, { id, name: pad.id, pad, device: { id, name: pad.id, standard, rumble }, sample: { deviceId: id, state: browserPadState(pad) } })
          changed = true
          continue
        }
        previous.pad = pad
        if (previous.device.standard !== standard || previous.device.rumble !== rumble) {
          previous.device = { ...previous.device, standard, rumble }
          previous.sample = { deviceId: previous.id, state: browserPadState(pad) }
          changed = true
        }
        else if (!padMatches(pad, previous.sample.state)) {
          // 状态保持不可变，消费者可以安全复用上一帧作为边沿基线。
          previous.sample = { deviceId: previous.id, state: browserPadState(pad) }
          changed = true
        }
      }
      for (const [index, device] of this.devices) {
        if (!this.seen.has(index)) {
          void this.stopRumble(device.id)
          this.devices.delete(index)
          changed = true
        }
      }
    }
    catch (failure) {
      error = failure instanceof Error ? failure.message : String(failure)
      for (const device of this.devices.values())
        void this.stopRumble(device.id)
      changed ||= this.devices.size > 0
      this.devices.clear()
    }
    this.readCost.record(this.environment.now() - observedAt)
    if (!changed && error === this.lastPublished?.error && observedAt - this.lastPublished.observedAt < SNAPSHOT_HEARTBEAT_MS)
      return
    this.publishedCount++
    const elapsed = observedAt - this.statsStartedAt
    if (!this.sampling || elapsed >= 1000) {
      this.sampling = { source: 'browserPoll', sampledAtMs: observedAt, pollCount: this.sequence, publishedCount: this.publishedCount, pollingHz: elapsed >= 1000 ? (this.sequence - this.statsPollCount) * 1000 / elapsed : undefined, publishedHz: elapsed >= 1000 ? (this.publishedCount - this.statsPublishedCount) * 1000 / elapsed : undefined, gap: this.sampleGap.snapshot(), read: this.readCost.snapshot(), dispatch: this.dispatchCost.snapshot() }
      this.statsStartedAt = observedAt
      this.statsPollCount = this.sequence
      this.statsPublishedCount = this.publishedCount
    }
    const devices: InputDevice[] = []
    const samples: DeviceSample[] = []
    for (const device of this.devices.values()) {
      devices.push(device.device)
      samples.push(device.sample)
    }
    const snapshot: GamepadSnapshot = { devices, samples, sequence: this.sequence, observedAt, error, sampling: this.sampling }
    this.lastPublished = snapshot
    const dispatchStartedAt = this.environment.now()
    this.listener?.(snapshot)
    this.dispatchCost.record(this.environment.now() - dispatchStartedAt)
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
