import type {
  GamepadRumbleEffectDto,
  GamepadRumbleResultDto,
  GamepadRumbleTargetDto,
  LogicalPadId,
} from '@shared/gamepad/contract'
import type { InputRuntimeConfig } from '../../domain/input'
import { rpc } from '../../../services/rpc'

const GAMEPAD_RUMBLE_REPORT_TYPE = 128
const GAMEPAD_RUMBLE_HEADER_SIZE = 2
const GAMEPAD_RUMBLE_PAYLOAD_SIZE = 11
const LOGICAL_PAD_IDS: LogicalPadId[] = ['pad-0', 'pad-1', 'pad-2', 'pad-3']

interface ParsedPacketRumble {
  target: GamepadRumbleTargetDto
  effect: GamepadRumbleEffectDto
  repeat: number
  cycleIntervalMs: number
}

interface TargetRumbleState {
  appliedEffectKey: string | null
  requestSeq: number
  packetEffect: GamepadRumbleEffectDto | null
  packetStopTimer: number | null
  packetRepeatTimer: number | null
}

interface BrowserRumbleActuator {
  playEffect?: (
    type: 'dual-rumble',
    params: {
      startDelay: number
      duration: number
      weakMagnitude: number
      strongMagnitude: number
    },
  ) => Promise<unknown>
  pulse?: (value: number, duration: number) => Promise<unknown>
  reset?: () => Promise<unknown>
}

interface BrowserRumbleGamepad {
  connected: boolean
  vibrationActuator?: BrowserRumbleActuator | null
  hapticActuators?: BrowserRumbleActuator[]
}

function createTargetState(): TargetRumbleState {
  return {
    appliedEffectKey: null,
    requestSeq: 0,
    packetEffect: null,
    packetStopTimer: null,
    packetRepeatTimer: null,
  }
}

function targetKey(target: GamepadRumbleTargetDto): string {
  return target.kind === 'logical-pad' ? `logical:${target.padId}` : `device:${target.deviceId}`
}

function logicalPadTargetFromIndex(gamepadIndex: number): GamepadRumbleTargetDto | null {
  const padId = LOGICAL_PAD_IDS[gamepadIndex]
  return padId === undefined
    ? null
    : {
        kind: 'logical-pad',
        padId,
      }
}

function quantizeMagnitude(value: number): number {
  const normalized = Math.max(0, Math.min(1, value))
  if (normalized < 0.05) {
    return 0
  }

  return Math.round(normalized * 16) / 16
}

function parsePacketRumble(
  payload: ArrayBuffer,
  runtime: InputRuntimeConfig,
): ParsedPacketRumble | null {
  if (!runtime.vibrationEnabled) {
    return null
  }

  const dataView = new DataView(payload)
  if (dataView.byteLength < GAMEPAD_RUMBLE_HEADER_SIZE + GAMEPAD_RUMBLE_PAYLOAD_SIZE) {
    return null
  }

  if (dataView.getUint8(0) !== GAMEPAD_RUMBLE_REPORT_TYPE) {
    return null
  }

  let offset = GAMEPAD_RUMBLE_HEADER_SIZE
  // 历史协议里这里有一个保留字节，旧逻辑只消费不使用。
  void dataView.getUint8(offset)

  const gamepadIndex = dataView.getUint8(offset + 1)
  const target = logicalPadTargetFromIndex(gamepadIndex)
  if (target === null) {
    return null
  }

  offset += 2
  const leftMotorPercent = dataView.getUint8(offset) / 100
  const rightMotorPercent = dataView.getUint8(offset + 1) / 100
  const leftTriggerMotorPercent = dataView.getUint8(offset + 2) / 100
  const rightTriggerMotorPercent = dataView.getUint8(offset + 3) / 100
  const rawDurationMs = dataView.getUint16(offset + 4, true)
  const delayMs = dataView.getUint16(offset + 6, true)
  const repeat = dataView.getUint8(offset + 8)

  return {
    target,
    effect: {
      startDelayMs: 0,
      // 旧实现会把协议值缩小十倍后再喂给浏览器 vibrationActuator。
      durationMs: Math.max(1, Math.round(rawDurationMs / 10)),
      strongMagnitude: quantizeMagnitude(leftMotorPercent),
      weakMagnitude: quantizeMagnitude(rightMotorPercent),
      leftTrigger: quantizeMagnitude(leftTriggerMotorPercent),
      rightTrigger: quantizeMagnitude(rightTriggerMotorPercent),
      repeat: 0,
    },
    repeat,
    cycleIntervalMs: Math.max(1, delayMs + rawDurationMs),
  }
}

/**
 * renderer 以 native rumble 为主。
 * 当宿主明确返回未实现时，再退回浏览器 haptics，避免 macOS 上完全无震动。
 */
export class RumbleService {
  private readonly targetStates = new Map<string, TargetRumbleState>()

  destroy(): void {
    for (const [key, state] of this.targetStates.entries()) {
      this.clearPacketTimers(state)
      state.packetEffect = null
      this.flushTargetEffectByKey(key, state)
    }
  }

  handlePacket(event: MessageEvent<ArrayBuffer>, runtime: InputRuntimeConfig): void {
    const parsed = parsePacketRumble(event.data, runtime)
    if (parsed === null) {
      return
    }

    this.playPacketEffect(parsed)
  }

  private playPacketEffect(parsed: ParsedPacketRumble): void {
    const state = this.stateForTarget(parsed.target)
    this.clearPacketTimers(state)
    this.startPacketPulse(parsed.target, state, parsed.effect)

    if (parsed.repeat <= 0) {
      return
    }

    let repeatCount = parsed.repeat
    state.packetRepeatTimer = window.setInterval(() => {
      if (repeatCount <= 0) {
        this.clearPacketRepeatTimer(state)
        this.pruneTargetState(parsed.target, state)
        return
      }

      this.startPacketPulse(parsed.target, state, parsed.effect)
      repeatCount -= 1
    }, parsed.cycleIntervalMs)
  }

  private startPacketPulse(
    target: GamepadRumbleTargetDto,
    state: TargetRumbleState,
    effect: GamepadRumbleEffectDto,
  ): void {
    this.clearPacketStopTimer(state)
    state.packetEffect = effect
    this.flushTargetEffect(target, state)
    state.packetStopTimer = window.setTimeout(() => {
      state.packetStopTimer = null
      state.packetEffect = null
      this.flushTargetEffect(target, state)
      this.pruneTargetState(target, state)
    }, effect.durationMs)
  }

  private stateForTarget(target: GamepadRumbleTargetDto): TargetRumbleState {
    const key = targetKey(target)
    let state = this.targetStates.get(key)
    if (state !== undefined) {
      return state
    }

    state = createTargetState()
    this.targetStates.set(key, state)
    return state
  }

  private flushTargetEffect(target: GamepadRumbleTargetDto, state: TargetRumbleState): void {
    this.flushTargetEffectByKey(targetKey(target), state)
  }

  private flushTargetEffectByKey(targetStateKey: string, state: TargetRumbleState): void {
    const nextEffect = state.packetEffect
    const nextKey = nextEffect === null ? null : JSON.stringify(nextEffect)
    if (nextKey === state.appliedEffectKey) {
      return
    }

    state.appliedEffectKey = nextKey
    const requestSeq = ++state.requestSeq
    const target = this.targetFromStateKey(targetStateKey)
    if (target === null) {
      return
    }
    void this.dispatchTargetEffect(target, nextEffect, requestSeq)
  }

  private targetFromStateKey(stateKey: string): GamepadRumbleTargetDto | null {
    if (stateKey.startsWith('logical:')) {
      const padId = stateKey.slice('logical:'.length) as LogicalPadId
      return LOGICAL_PAD_IDS.includes(padId)
        ? {
            kind: 'logical-pad',
            padId,
          }
        : null
    }

    if (stateKey.startsWith('device:')) {
      return {
        kind: 'device',
        deviceId: stateKey.slice('device:'.length),
      }
    }

    return null
  }

  private async dispatchTargetEffect(
    target: GamepadRumbleTargetDto,
    effect: GamepadRumbleEffectDto | null,
    requestSeq: number,
  ): Promise<void> {
    try {
      const result = effect === null
        ? await rpc.gamepad.stopRumble({ target })
        : await rpc.gamepad.playRumble({
            request: {
              target,
              effect,
            },
          })

      if (result.accepted) {
        return
      }

      const fallbackApplied = await this.tryBrowserRumbleFallback(target, effect, result)
      const state = this.targetStates.get(targetKey(target))
      if (state && requestSeq === state.requestSeq && !fallbackApplied) {
        console.warn('[player][rumble] native rumble unavailable', {
          target,
          effect,
          reason: result.reason,
          resolvedDeviceIds: result.resolvedDeviceIds,
        })
      }
    }
    catch (error) {
      const fallbackApplied = await this.tryBrowserRumbleFallback(target, effect, null)
      const state = this.targetStates.get(targetKey(target))
      if (state && requestSeq === state.requestSeq && !fallbackApplied) {
        console.warn('[player][rumble] native rumble request failed', error)
      }
    }
  }

  private async tryBrowserRumbleFallback(
    target: GamepadRumbleTargetDto,
    effect: GamepadRumbleEffectDto | null,
    result: GamepadRumbleResultDto | null,
  ): Promise<boolean> {
    if (result !== null && !this.shouldUseBrowserFallback(result)) {
      return false
    }

    try {
      if (effect === null) {
        return await stopBrowserRumble(target)
      }

      return await playBrowserRumble(target, effect)
    }
    catch (error) {
      console.warn('[player][rumble] browser rumble fallback failed', {
        target,
        effect,
        error,
      })
      return false
    }
  }

  private shouldUseBrowserFallback(result: GamepadRumbleResultDto): boolean {
    return result.reason === 'not-implemented' || result.reason === 'unsupported'
  }

  private pruneTargetState(target: GamepadRumbleTargetDto, state: TargetRumbleState): void {
    if (
      state.packetEffect !== null
      || state.packetStopTimer !== null
      || state.packetRepeatTimer !== null
    ) {
      return
    }

    this.targetStates.delete(targetKey(target))
  }

  private clearPacketTimers(state: TargetRumbleState): void {
    this.clearPacketStopTimer(state)
    this.clearPacketRepeatTimer(state)
  }

  private clearPacketStopTimer(state: TargetRumbleState): void {
    if (state.packetStopTimer !== null) {
      window.clearTimeout(state.packetStopTimer)
      state.packetStopTimer = null
    }
  }

  private clearPacketRepeatTimer(state: TargetRumbleState): void {
    if (state.packetRepeatTimer !== null) {
      window.clearInterval(state.packetRepeatTimer)
      state.packetRepeatTimer = null
    }
  }
}

function getBrowserGamepad(target: GamepadRumbleTargetDto): BrowserRumbleGamepad | null {
  if (target.kind !== 'logical-pad') {
    return null
  }

  if (typeof navigator.getGamepads !== 'function') {
    return null
  }

  const gamepadIndex = LOGICAL_PAD_IDS.indexOf(target.padId)
  if (gamepadIndex < 0) {
    return null
  }

  const gamepad = navigator.getGamepads()[gamepadIndex] as BrowserRumbleGamepad | null
  return gamepad?.connected ? gamepad : null
}

function clampMagnitude(value: number): number {
  return Math.max(0, Math.min(1, value))
}

function resolveBrowserActuator(
  gamepad: BrowserRumbleGamepad,
): BrowserRumbleActuator | null {
  return gamepad.vibrationActuator ?? gamepad.hapticActuators?.[0] ?? null
}

async function playBrowserRumble(
  target: GamepadRumbleTargetDto,
  effect: GamepadRumbleEffectDto,
): Promise<boolean> {
  const gamepad = getBrowserGamepad(target)
  if (gamepad === null) {
    return false
  }

  const actuator = resolveBrowserActuator(gamepad)
  if (typeof actuator?.playEffect === 'function') {
    await actuator.playEffect('dual-rumble', {
      startDelay: effect.startDelayMs,
      duration: Math.max(1, effect.durationMs),
      strongMagnitude: clampMagnitude(Math.max(effect.strongMagnitude, effect.leftTrigger)),
      weakMagnitude: clampMagnitude(Math.max(effect.weakMagnitude, effect.rightTrigger)),
    })
    return true
  }

  if (typeof actuator?.pulse === 'function') {
    const value = clampMagnitude(
      Math.max(
        effect.strongMagnitude,
        effect.weakMagnitude,
        effect.leftTrigger,
        effect.rightTrigger,
      ),
    )
    await actuator.pulse(value, Math.max(1, effect.durationMs))
    return true
  }

  return false
}

async function stopBrowserRumble(target: GamepadRumbleTargetDto): Promise<boolean> {
  const gamepad = getBrowserGamepad(target)
  if (gamepad === null) {
    return false
  }

  const actuator = resolveBrowserActuator(gamepad)
  if (typeof actuator?.reset === 'function') {
    await actuator.reset()
    return true
  }

  if (typeof actuator?.playEffect === 'function') {
    await actuator.playEffect('dual-rumble', {
      startDelay: 0,
      duration: 1,
      strongMagnitude: 0,
      weakMagnitude: 0,
    })
    return true
  }

  if (typeof actuator?.pulse === 'function') {
    await actuator.pulse(0, 1)
    return true
  }

  return false
}
