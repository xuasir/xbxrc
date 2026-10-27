import type { GamepadBackend, GamepadSnapshot, InputDevice, RumbleResult } from './backend'
import type { GamepadRumbleEffectDto, LogicalPadStateDto } from './contract'
import type { GamepadPreferences, InputOwner } from './preferences'
import { emptyGamepadSnapshot } from './backend'
import { applyGameControls, deriveInputOwner, parseGamepadPreferences } from './preferences'
import { gameRumbleEffect } from './rumble'
import { BUTTON_KEYS, clonePadState, neutralState, PadReleaseBarrier } from './state'

export interface InputSnapshot {
  backend: GamepadBackend['kind'] | null
  preferences: GamepadPreferences
  devices: InputDevice[]
  activeDeviceId: string | null
  owner: InputOwner
  streamReady: boolean
  uiCapturing: boolean
  exclusiveCapture: boolean
  isBackground: boolean | undefined
  physicalState: LogicalPadStateDto
  uiState: LogicalPadStateDto
  gameState: LogicalPadStateDto
  sequence: number
  observedAt: number
  error: string | null
}

/** 设备源与媒体执行解耦；所有消费者读取同一次归属决策。 */
export class InputController {
  private backend: GamepadBackend | undefined
  private preferences = parseGamepadPreferences({})
  private raw = emptyGamepadSnapshot()
  private initializationError: string | null = null
  private selectedId: string | null = null
  private streamReady = false
  private isBackground: boolean | undefined
  private captures = new Map<string, boolean>()
  private keyboard = neutralState()
  private virtual = neutralState()
  private systemHome = false
  private uiBarrier = new PadReleaseBarrier()
  private gameBarrier = new PadReleaseBarrier()
  private owner: InputOwner = 'ui'
  private generation = 0
  private starting: Promise<void> | undefined
  private stopping: Promise<void> | undefined
  private listeners = new Set<(snapshot: InputSnapshot) => void>()
  private menuListeners = new Set<() => void>()
  private comboHeld = false
  private publishing = false
  private revision = 0
  private current: InputSnapshot = this.buildSnapshot()

  getSnapshot(): InputSnapshot {
    return this.current
  }

  subscribe(listener: (snapshot: InputSnapshot) => void): () => void {
    this.listeners.add(listener)
    this.notify(listener, this.current)
    return () => this.listeners.delete(listener)
  }

  onMenuRequested(listener: () => void): () => void {
    this.menuListeners.add(listener)
    return () => this.menuListeners.delete(listener)
  }

  setInitializationError(error: string | null): void {
    this.initializationError = error
    this.publish()
  }

  async start(backend: GamepadBackend): Promise<void> {
    if (this.stopping)
      await this.stopping
    if (this.backend)
      return this.starting
    const generation = ++this.generation
    this.backend = backend
    this.publish()
    this.starting = backend.start((snapshot) => {
      if (generation === this.generation)
        this.accept(snapshot)
    })
    try {
      await this.starting
    }
    catch (error) {
      if (generation === this.generation) {
        ++this.generation
        this.backend = undefined
        await backend.stop().catch(cleanupError => console.warn('[gamepad] startup cleanup failed', cleanupError))
        this.raw = { ...emptyGamepadSnapshot(), error: error instanceof Error ? error.message : String(error) }
        this.publish()
      }
      throw error
    }
  }

  async stop(): Promise<void> {
    if (this.stopping)
      return this.stopping
    ++this.generation
    const backend = this.backend
    this.backend = undefined
    this.raw = emptyGamepadSnapshot()
    this.selectedId = null
    this.streamReady = false
    this.captures.clear()
    this.keyboard = neutralState()
    this.virtual = neutralState()
    this.systemHome = false
    this.comboHeld = false
    this.rearm()
    this.publish()
    // 初始化可能仍在等待原生监听注册；完成后再关闭，防止迟到的监听泄漏。
    const starting = this.starting
    this.stopping = (async () => {
      await starting?.catch(() => {})
      await backend?.stop()
    })()
    try {
      await this.stopping
    }
    finally {
      this.stopping = undefined
      this.starting = undefined
    }
  }

  configure(preferences: GamepadPreferences): void {
    const old = this.preferences
    this.preferences = preferences
    if (old.controlSource !== preferences.controlSource
      || JSON.stringify(old.gameControls) !== JSON.stringify(preferences.gameControls)) {
      this.rearm()
    }
    if ((old.vibration && !preferences.vibration) || (old.uiHaptics && !preferences.uiHaptics))
      void this.stopRumble()
    this.publish()
  }

  setBackground(isBackground: boolean | undefined): void {
    if (this.isBackground === isBackground)
      return
    this.isBackground = isBackground
    this.publish()
  }

  setStreamReady(ready: boolean): void {
    if (this.streamReady === ready)
      return
    this.streamReady = ready
    if (!ready) {
      this.virtual = neutralState()
      this.systemHome = false
    }
    this.publish()
  }

  setUiCapture(key: string, active: boolean, exclusive = false): void {
    if (active ? this.captures.get(key) === exclusive : !this.captures.has(key))
      return
    if (active)
      this.captures.set(key, exclusive)
    else
      this.captures.delete(key)
    this.rearm()
    this.publish()
  }

  selectDevice(id: string): boolean {
    if (!this.raw.devices.some(device => device.id === id && device.standard))
      return false
    if (this.selectedId !== id) {
      void this.stopRumble()
      this.selectedId = id
      this.comboHeld = false
      this.rearm()
      this.publish()
    }
    return true
  }

  setKeyboardState(state: LogicalPadStateDto): void {
    this.keyboard = clonePadState(state)
    if (this.preferences.controlSource === 'keyboard')
      this.checkMenuCombo(true)
    this.publish()
  }

  setVirtualState(state: LogicalPadStateDto): void {
    this.virtual = clonePadState(state)
    this.publish()
  }

  setSystemHome(pressed: boolean): void {
    this.systemHome = pressed
    this.publish()
  }

  async rumble(effect: GamepadRumbleEffectDto, source: 'game' | 'ui' | 'test'): Promise<RumbleResult> {
    const { owner, activeDeviceId } = this.current
    if (!this.backend || !activeDeviceId || owner === 'blocked')
      return 'unsupported'
    if (source === 'game' && (owner !== 'stream' || !this.preferences.vibration || this.preferences.controlSource === 'keyboard'))
      return 'unsupported'
    if (source === 'ui' && (owner !== 'ui' || !this.preferences.uiHaptics || this.current.exclusiveCapture))
      return 'unsupported'
    if (source === 'test' && owner !== 'ui')
      return 'unsupported'
    return this.backend.rumble(activeDeviceId, source === 'ui' ? effect : gameRumbleEffect(effect, this.preferences.vibrationStrength))
  }

  async stopRumble(): Promise<void> {
    if (this.backend && this.selectedId)
      await this.backend.stopRumble(this.selectedId)
  }

  private physical(): LogicalPadStateDto {
    return this.raw.samples.find(sample => sample.deviceId === this.selectedId)?.state ?? neutralState()
  }

  private gameInput(): LogicalPadStateDto {
    const state = clonePadState(this.preferences.controlSource === 'keyboard' ? this.keyboard : this.physical())
    for (const key of BUTTON_KEYS)
      state.buttons[key] = Math.max(state.buttons[key], this.virtual.buttons[key])
    for (const key of ['leftStick', 'rightStick'] as const) {
      if (this.virtual[key].x || this.virtual[key].y)
        state[key] = { ...this.virtual[key] }
    }
    state.leftTrigger = Math.max(state.leftTrigger, this.virtual.leftTrigger)
    state.rightTrigger = Math.max(state.rightTrigger, this.virtual.rightTrigger)
    return state
  }

  private rearm(): void {
    this.uiBarrier.capture(this.physical())
    this.gameBarrier = new PadReleaseBarrier(
      this.preferences.gameControls.leftDeadzone,
      this.preferences.gameControls.rightDeadzone,
    )
    this.gameBarrier.capture(this.gameInput())
  }

  private accept(snapshot: GamepadSnapshot): void {
    const previous = this.raw
    const previousId = this.selectedId
    const previouslyConnected = previous.devices.some(device => device.id === previousId)
    this.raw = snapshot
    const devices = snapshot.devices.filter(device => device.standard)
    if (this.selectedId === null && devices.length === 1) {
      this.selectedId = devices[0].id
    }
    else if (!devices.some(device => device.id === this.selectedId)) {
      // 失去当前设备后，由另一台设备的新按键明确取得输入权。
      const activated = devices.find((device) => {
        const before = previous.samples.find(sample => sample.deviceId === device.id)?.state
        const after = snapshot.samples.find(sample => sample.deviceId === device.id)?.state
        return before && after && BUTTON_KEYS.some(key => key !== 'l2' && key !== 'r2'
          && before.buttons[key] <= 0.5 && after.buttons[key] > 0.5)
      })
      if (activated)
        this.selectedId = activated.id
    }
    if (previousId !== this.selectedId || (previouslyConnected && !devices.some(device => device.id === previousId))) {
      if (previousId)
        void this.backend?.stopRumble(previousId)
      this.rearm()
    }
    this.checkMenuCombo(previousId === this.selectedId && previouslyConnected)
    this.publish()
  }

  private checkMenuCombo(allowTrigger: boolean): void {
    const sources = [this.physical()]
    if (this.preferences.controlSource === 'keyboard')
      sources.push(this.keyboard)
    if (sources.every(state => state.buttons.menu <= 0.5 && state.buttons.view <= 0.5))
      this.comboHeld = false
    if (sources.some(state => state.buttons.menu > 0.5 && state.buttons.view > 0.5) && !this.comboHeld) {
      this.comboHeld = true
      if (allowTrigger && this.owner !== 'blocked' && !this.current.exclusiveCapture) {
        for (const listener of this.menuListeners) this.notify(listener, undefined)
      }
    }
  }

  private buildSnapshot(): InputSnapshot {
    const exclusive = [...this.captures.values()].some(Boolean)
    const physicalState = clonePadState(this.physical())
    const ui = this.uiBarrier.apply(physicalState)
    const gameInput = this.gameBarrier.apply(this.gameInput())
    if (this.comboHeld) {
      ui.buttons.menu = ui.buttons.view = 0
      gameInput.buttons.menu = gameInput.buttons.view = 0
    }
    const game = applyGameControls(gameInput, this.preferences.gameControls)
    const gameState = this.owner === 'stream' ? game : neutralState()
    if (this.streamReady && this.owner !== 'blocked' && !exclusive && this.systemHome)
      gameState.buttons.home = 1
    return {
      backend: this.backend?.kind ?? null,
      preferences: this.preferences,
      devices: this.raw.devices,
      activeDeviceId: this.raw.devices.some(device => device.id === this.selectedId) ? this.selectedId : null,
      owner: this.owner,
      streamReady: this.streamReady,
      uiCapturing: this.captures.size > 0,
      exclusiveCapture: exclusive,
      isBackground: this.isBackground,
      physicalState,
      uiState: this.owner === 'ui' && !exclusive ? ui : neutralState(),
      gameState,
      sequence: this.raw.sequence,
      observedAt: this.raw.observedAt,
      error: this.raw.error ?? this.initializationError,
    }
  }

  private publish(): void {
    const owner = deriveInputOwner(this.preferences.backgroundPolicy, this.isBackground, this.streamReady, this.captures.size > 0)
    if (owner !== this.owner) {
      void this.stopRumble()
      if (owner === 'blocked')
        this.systemHome = false
      this.owner = owner
      this.rearm()
    }
    this.current = this.buildSnapshot()
    ++this.revision
    if (this.publishing)
      return
    this.publishing = true
    try {
      let delivered: number
      do {
        delivered = this.revision
        const snapshot = this.current
        for (const listener of this.listeners) {
          this.notify(listener, snapshot)
          // 点击/菜单回调可以同步改变归属，其余消费者应看到新的决策。
          if (this.revision !== delivered)
            break
        }
      } while (delivered !== this.revision)
    }
    finally {
      this.publishing = false
    }
  }

  private notify<T>(listener: (value: T) => void, value: T): void {
    try {
      listener(value)
    }
    catch (error) {
      console.error('[gamepad] input consumer failed', error)
    }
  }
}
