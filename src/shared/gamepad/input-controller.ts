import type { DeviceSample, GamepadBackend, GamepadSnapshot, InputDevice, InputSamplingObservation, RawPadState, RumbleResult } from './backend'
import type { GamepadRumbleEffectDto, LogicalPadStateDto } from './contract'
import type { GamepadPreferences, InputOwner } from './preferences'
import { emptyGamepadSnapshot } from './backend'
import { applyDeviceMapping, defaultDeviceMapping, deviceAxisRest } from './device-mapping'
import { applyGameControls, defaultGameControls, deriveInputOwner, parseGamepadPreferences } from './preferences'
import { RawButtonReleaseBarrier } from './raw-input'
import { gameRumbleEffect } from './rumble'
import { RumbleScheduler } from './rumble-scheduler'
import { BUTTON_KEYS, clonePadState, mergePadStates, neutralState, PadReleaseBarrier } from './state'

export interface InputSnapshot {
  backend: GamepadBackend['kind'] | null
  preferences: GamepadPreferences
  devices: InputDevice[]
  activeDeviceId: string | null
  owner: InputOwner
  streamReady: boolean
  uiCapturing: boolean
  exclusiveCapture: boolean
  keyboardObserving: boolean
  keyboardState: LogicalPadStateDto
  isBackground: boolean | undefined
  physicalState: LogicalPadStateDto
  physicalRaw?: RawPadState
  deviceSamples: DeviceSample[]
  uiState: LogicalPadStateDto
  gameState: LogicalPadStateDto
  sequence: number
  observedAt: number
  error: string | null
  sampling?: InputSamplingObservation
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
  private keyboardObservers = new Set<string>()
  private keyboard = neutralState()
  private keyboardUi = neutralState()
  private virtual = neutralState()
  private systemHome = false
  private uiBarrier = new PadReleaseBarrier()
  private keyboardUiBarrier = new PadReleaseBarrier()
  private gameBarrier = new PadReleaseBarrier()
  private virtualBarrier = new PadReleaseBarrier()
  private rawGameBarrier = new RawButtonReleaseBarrier()
  private owner: InputOwner = 'ui'
  private readonly rumbleScheduler = new RumbleScheduler()
  private generation = 0
  private starting: Promise<void> | undefined
  private stopping: Promise<void> | undefined
  private listeners = new Set<(snapshot: InputSnapshot) => void>()
  private menuListeners = new Set<(target: 'menu' | 'diagnostics') => void>()
  private comboHeld = false
  private diagnosticsComboHeld = false
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

  onMenuRequested(listener: (target: 'menu' | 'diagnostics') => void): () => void {
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
    this.rumbleScheduler.cancel()
    const backend = this.backend
    this.backend = undefined
    this.raw = emptyGamepadSnapshot()
    this.selectedId = null
    this.streamReady = false
    this.captures.clear()
    this.keyboard = neutralState()
    this.keyboardUi = neutralState()
    this.virtual = neutralState()
    this.systemHome = false
    this.comboHeld = false
    this.diagnosticsComboHeld = false
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
    if (JSON.stringify(old.streamShortcuts) !== JSON.stringify(preferences.streamShortcuts)) {
      this.comboHeld = this.diagnosticsComboHeld = false
      this.rearm()
    }
    if ((old.vibration && !preferences.vibration) || (old.uiHaptics && !preferences.uiHaptics)
      || old.controlSource !== preferences.controlSource) {
      void this.stopRumble()
    }
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

  setKeyboardObservation(key: string, active: boolean): void {
    if (this.keyboardObservers.has(key) === active)
      return
    if (active)
      this.keyboardObservers.add(key)
    else this.keyboardObservers.delete(key)
    this.publish()
  }

  selectDevice(id: string): boolean {
    if (!this.raw.devices.some(device => device.id === id))
      return false
    if (this.selectedId !== id) {
      void this.stopRumble()
      this.selectedId = id
      this.comboHeld = false
      this.diagnosticsComboHeld = false
      this.rearm()
      this.publish()
    }
    return true
  }

  setKeyboardState(state: LogicalPadStateDto, uiState: LogicalPadStateDto = state): void {
    this.keyboard = clonePadState(state)
    this.keyboardUi = clonePadState(uiState)
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
    const backend = this.backend
    if (source === 'game') {
      return this.rumbleScheduler.play(effect, value => backend.rumble(activeDeviceId, gameRumbleEffect(value, this.preferences.vibrationStrength, this.preferences.vibrationChannels)))
    }
    this.rumbleScheduler.cancel()
    return backend.rumble(activeDeviceId, source === 'ui' ? effect : gameRumbleEffect(effect, this.preferences.vibrationStrength, this.preferences.vibrationChannels))
  }

  async stopRumble(): Promise<void> {
    this.rumbleScheduler.cancel()
    if (this.backend && this.selectedId)
      await this.backend.stopRumble(this.selectedId)
  }

  private physical(): LogicalPadStateDto {
    const sample = this.raw.samples.find(sample => sample.deviceId === this.selectedId)
    const device = this.raw.devices.find(device => device.id === this.selectedId)
    const mapping = this.preferences.gameControls.deviceMappings.find(item => item.layout === device?.layoutKey) ?? (device && !device.standard ? defaultDeviceMapping(device.layoutKey ?? '', false, this.preferences.gameControls.buttons) : undefined)
    return device && !device.standard && mapping ? applyDeviceMapping(sample?.state ?? neutralState(), sample?.raw, mapping, false) : sample?.state ?? neutralState()
  }

  private gameInput(): LogicalPadStateDto {
    return this.preferences.controlSource === 'keyboard' ? this.keyboard : this.physical()
  }

  private keyboardUiInput(): LogicalPadStateDto {
    return this.preferences.controlSource === 'keyboard' ? this.keyboardUi : neutralState()
  }

  private rearm(): void {
    this.uiBarrier.capture(this.physical())
    // 各来源独立归位，键盘持键期间物理手柄的新按键仍可导航。
    this.keyboardUiBarrier.capture(this.keyboardUiInput())
    this.gameBarrier = new PadReleaseBarrier(
      this.preferences.gameControls.leftDeadzone,
      this.preferences.gameControls.rightDeadzone,
    )
    this.gameBarrier.capture(this.gameInput())
    this.virtualBarrier.capture(this.virtual)
    const device = this.raw.devices.find(device => device.id === this.selectedId)
    const mapping = this.preferences.gameControls.deviceMappings.find(item => item.layout === device?.layoutKey) ?? (device?.standard ? defaultDeviceMapping(device.layoutKey ?? '', true, this.preferences.gameControls.buttons) : undefined)
    const thresholds = new Map<number, number>()
    for (const binding of mapping?.bindings ?? []) {
      if (binding.kind === 'axis')
        thresholds.set(binding.axis, binding.range === 'signed' ? Math.max(0.03, binding.target.startsWith('left') ? this.preferences.gameControls.leftDeadzone : this.preferences.gameControls.rightDeadzone) : binding.range === 'full' ? 0.06 : 0.03)
    }
    this.rawGameBarrier.capture(this.raw.samples.find(sample => sample.deviceId === this.selectedId)?.raw, deviceAxisRest(mapping), thresholds)
  }

  private accept(snapshot: GamepadSnapshot): void {
    const previous = this.raw
    const previousId = this.selectedId
    const previouslyConnected = previous.devices.some(device => device.id === previousId)
    this.raw = snapshot
    const devices = snapshot.devices
    if (this.selectedId === null && devices.length === 1) {
      this.selectedId = devices[0].id
    }
    else if (!devices.some(device => device.id === this.selectedId)) {
      // 失去当前设备后，由另一台设备的新按键明确取得输入权。
      const activated = devices.find((device) => {
        const before = previous.samples.find(sample => sample.deviceId === device.id)?.state
        const after = snapshot.samples.find(sample => sample.deviceId === device.id)?.state
        const beforeRaw = previous.samples.find(sample => sample.deviceId === device.id)?.raw
        const afterRaw = snapshot.samples.find(sample => sample.deviceId === device.id)?.raw
        return before && after && (BUTTON_KEYS.some(key => key !== 'l2' && key !== 'r2' && before.buttons[key] <= 0.5 && after.buttons[key] > 0.5) || !!afterRaw?.buttons.some((value, index) => value > 0.5 && (beforeRaw?.buttons[index] ?? 0) <= 0.5))
      })
      if (activated)
        this.selectedId = activated.id
    }
    const layoutChanged = previous.devices.find(device => device.id === previousId)?.layoutKey
      !== snapshot.devices.find(device => device.id === this.selectedId)?.layoutKey
      || previous.devices.find(device => device.id === previousId)?.standard !== snapshot.devices.find(device => device.id === this.selectedId)?.standard
    if (previousId !== this.selectedId || layoutChanged || (previouslyConnected && !devices.some(device => device.id === previousId))) {
      this.rumbleScheduler.cancel()
      if (previousId)
        void this.backend?.stopRumble(previousId)
      this.rearm()
    }
    this.checkMenuCombo(previousId === this.selectedId && previouslyConnected)
    this.publish()
  }

  private checkMenuCombo(allowTrigger: boolean): void {
    const device = this.raw.devices.find(device => device.id === this.selectedId)
    const raw = this.raw.samples.find(sample => sample.deviceId === this.selectedId)?.raw
    const sources = [this.streamReady ? applyGameControls(this.physical(), this.preferences.gameControls, raw, device?.layoutKey, device?.standard) : this.physical()]
    if (this.preferences.controlSource === 'keyboard')
      sources.push(this.keyboard)
    for (const target of ['menu', 'diagnostics'] as const) {
      const keys = this.preferences.streamShortcuts[target]
      const field = target === 'menu' ? 'comboHeld' : 'diagnosticsComboHeld'
      if (!keys.length || sources.every(state => keys.every(key => state.buttons[key] <= 0.5)))
        this[field] = false
      if (keys.length && sources.some(state => keys.every(key => state.buttons[key] > 0.5)) && !this[field]) {
        this[field] = true
        if (allowTrigger && this.owner !== 'blocked' && !this.current.exclusiveCapture) {
          for (const listener of this.menuListeners) this.notify(listener, target)
        }
      }
    }
  }

  private buildSnapshot(): InputSnapshot {
    const exclusive = [...this.captures.values()].some(Boolean)
    const physicalState = clonePadState(this.physical())
    const ui = mergePadStates(this.uiBarrier.apply(physicalState), this.keyboardUiBarrier.apply(this.keyboardUiInput()))
    const gameInput = this.gameBarrier.apply(this.gameInput())
    const raw = this.raw.samples.find(sample => sample.deviceId === this.selectedId)?.raw
    const device = this.raw.devices.find(device => device.id === this.selectedId)
    const game = mergePadStates(applyGameControls(gameInput, this.preferences.controlSource === 'keyboard' ? defaultGameControls() : this.preferences.gameControls, this.preferences.controlSource === 'gamepad' ? this.rawGameBarrier.apply(raw) : undefined, device?.layoutKey, device?.standard), this.virtualBarrier.apply(this.virtual))
    const consumed = [...(this.comboHeld ? this.preferences.streamShortcuts.menu : []), ...(this.diagnosticsComboHeld ? this.preferences.streamShortcuts.diagnostics : [])]
    for (const key of consumed) {
      ui.buttons[key] = game.buttons[key] = 0
      if (key === 'l2')
        ui.leftTrigger = game.leftTrigger = 0
      if (key === 'r2')
        ui.rightTrigger = game.rightTrigger = 0
    }
    const gameState = this.owner === 'stream' ? game : neutralState()
    if (this.streamReady && this.owner !== 'blocked' && !exclusive && this.systemHome)
      gameState.buttons.home = 1
    return {
      backend: this.backend?.kind ?? null,
      preferences: this.preferences,
      devices: this.raw.devices,
      activeDeviceId: device ? this.selectedId : null,
      owner: this.owner,
      streamReady: this.streamReady,
      uiCapturing: this.captures.size > 0,
      exclusiveCapture: exclusive,
      keyboardObserving: this.keyboardObservers.size > 0,
      keyboardState: clonePadState(this.keyboard),
      isBackground: this.isBackground,
      physicalState,
      physicalRaw: raw ? { buttons: [...raw.buttons], axes: [...raw.axes] } : undefined,
      deviceSamples: this.raw.samples,
      uiState: this.owner === 'ui' && !exclusive ? ui : neutralState(),
      gameState,
      sequence: this.raw.sequence,
      observedAt: this.raw.observedAt,
      sampling: this.raw.sampling,
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
