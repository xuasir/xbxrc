import type { LogicalButtonDto } from '@shared/gamepad/contract'
import type { InputDriverLike } from '../app/input/InputService'
import type { PlayerClientOptions } from '../domain/config'
import type { GamepadFrame, InputRuntimeConfig } from '../domain/input'
import type { AudioRuntimeConfig, RendererRuntimeConfig, StreamStats } from '../domain/media'
import type {
  ConnectParams,
  CreateOfferOptions,
  IceCandidateLike,
  SessionState,
  TransportRuntimeConfig,
} from '../domain/session'
import type { PlayerEvents } from './events'
import { InputService } from '../app/input/InputService'
import { MediaService } from '../app/media/MediaService'
import { SessionService } from '../app/session/SessionService'
import { StatsService } from '../app/stats/StatsService'
import { DEFAULT_PLAYER_OPTIONS } from '../domain/config'
import { GamepadDriver } from '../infra/input/GamepadDriver'
import { TypedEventEmitter } from './events'

type PlayerInputDriver = InputDriverLike & {
  setApplication?: (app: PlayerClient) => void
}

export interface PlayerClientInit extends Omit<
  Partial<PlayerClientOptions>,
  'input' | 'audio' | 'renderer' | 'transport'
> {
  input?: Partial<InputRuntimeConfig>
  audio?: Partial<AudioRuntimeConfig>
  renderer?: Partial<RendererRuntimeConfig>
  transport?: Partial<TransportRuntimeConfig>
}

export interface PlayerInputController {
  updateConfig: (config: Partial<InputRuntimeConfig>) => void
  setGamepadState: (state: GamepadFrame) => void
  pressButton: (button: LogicalButtonDto, durationMs: number) => void
  pressButtonStart: (button: LogicalButtonDto) => void
  pressButtonEnd: (button: LogicalButtonDto) => void
  moveLeftStick: (x: number, y: number) => void
  moveRightStick: (x: number, y: number) => void
}

export interface PlayerAudioController {
  updateConfig: (config: Partial<AudioRuntimeConfig>) => void
  setVolumeDirect: (value: number) => void
  startMic: () => Promise<void>
  stopMic: () => Promise<void>
  getMicState: () => { capturing: boolean, paused: boolean }
}

export interface PlayerStatsController {
  snapshot: () => Promise<StreamStats>
  subscribe: (listener: (stats: StreamStats) => void) => () => void
}

export class PlayerClient {
  private readonly emitter = new TypedEventEmitter<PlayerEvents>()
  private readonly gamepadDriver: InputDriverLike
  private readonly inputService: InputService
  private readonly mediaService: MediaService
  private readonly sessionService: SessionService
  private readonly statsService: StatsService
  private options: PlayerClientOptions

  constructor(options: PlayerClientInit) {
    const defaults = DEFAULT_PLAYER_OPTIONS()
    this.options = {
      ...defaults,
      ...options,
      input: { ...defaults.input, ...(options.input ?? {}) },
      audio: { ...defaults.audio, ...(options.audio ?? {}) },
      renderer: { ...defaults.renderer, ...(options.renderer ?? {}) },
      transport: { ...defaults.transport, ...(options.transport ?? {}) },
      uiSystem: options.uiSystem ?? defaults.uiSystem,
      uiVersion: options.uiVersion ?? defaults.uiVersion,
    }
    const nextInputDriver = this.resolveInputDriverOption(options.inputDriver)
    this.gamepadDriver = this.createInputDriver(nextInputDriver)
    this.inputService = new InputService(this.options.input, this.gamepadDriver, this.emitter)
    if (nextInputDriver && typeof nextInputDriver.setApplication === 'function') {
      nextInputDriver.setApplication(this)
    }
    this.mediaService = new MediaService(
      () => this.resolveContainer(),
      this.options.audio,
      this.options.renderer,
      this.inputService,
      this.emitter,
    )
    this.sessionService = new SessionService(
      this.options,
      this.inputService,
      this.mediaService,
      this.emitter,
    )
    this.statsService = new StatsService(() => this.sessionService.getPeer(), this.emitter)
    this.statsService.start()
  }

  events(): TypedEventEmitter<PlayerEvents> {
    return this.emitter
  }

  getState(): SessionState {
    return this.sessionService.getState()
  }

  bind(params?: ConnectParams): void {
    this.sessionService.bind(params?.turnServer)
  }

  async createOffer(options?: CreateOfferOptions): Promise<RTCSessionDescriptionInit> {
    const offer = await this.sessionService.createOffer(options)
    return offer
  }

  async setRemoteDescription(answerSdp: string): Promise<void> {
    await this.sessionService.setRemoteAnswer(answerSdp)
  }

  async addIceCandidates(candidates: Array<IceCandidateLike>): Promise<void> {
    await this.sessionService.addIceCandidates(candidates)
  }

  getIceCandidates(): Array<IceCandidateLike> {
    return this.sessionService.getIceCandidates()
  }

  getPeer(): RTCPeerConnection | undefined {
    return this.sessionService.getPeer()
  }

  async waitForIceCandidates(timeoutMs = 4000): Promise<Array<IceCandidateLike>> {
    const peer = this.sessionService.getPeer()
    if (!peer || peer.iceGatheringState === 'complete') {
      return this.getIceCandidates()
    }

    return await new Promise<Array<IceCandidateLike>>((resolve) => {
      let settled = false
      let quietTimerId: number | null = null

      const clearQuietTimer = (): void => {
        if (quietTimerId !== null) {
          window.clearTimeout(quietTimerId)
          quietTimerId = null
        }
      }

      const finish = (): void => {
        if (settled) {
          return
        }
        settled = true
        clearQuietTimer()
        window.clearTimeout(timeoutId)
        peer.removeEventListener('icecandidate', handleIceCandidate)
        peer.removeEventListener('icegatheringstatechange', handleGatheringStateChange)
        resolve(this.getIceCandidates())
      }

      const scheduleQuietFinish = (): void => {
        if (this.getIceCandidates().length === 0) {
          return
        }
        clearQuietTimer()
        // 收到首批候选后只等一个短静默窗口，避免仍然被固定 4s gather timeout 拖慢。
        quietTimerId = window.setTimeout(finish, 150)
      }

      const handleIceCandidate = (event: RTCPeerConnectionIceEvent): void => {
        if (event.candidate === null) {
          finish()
          return
        }
        scheduleQuietFinish()
      }

      const handleGatheringStateChange = (): void => {
        if (peer.iceGatheringState === 'complete') {
          finish()
        }
      }

      const timeoutId = window.setTimeout(finish, timeoutMs)
      peer.addEventListener('icecandidate', handleIceCandidate)
      peer.addEventListener('icegatheringstatechange', handleGatheringStateChange)
      scheduleQuietFinish()
    })
  }

  updateInputConfig(config: Partial<InputRuntimeConfig>): void {
    this.options.input = { ...this.options.input, ...config }
    this.inputService.updateRuntime(config)
  }

  updateAudioConfig(config: Partial<AudioRuntimeConfig>): void {
    this.options.audio = { ...this.options.audio, ...config }
    this.mediaService.updateAudioConfig(config)
  }

  updateTransportConfig(config: Partial<TransportRuntimeConfig>): void {
    this.options.transport = { ...this.options.transport, ...config }
  }

  updateRenderer(config: Partial<RendererRuntimeConfig>): void {
    this.options.renderer = { ...this.options.renderer, ...config }
    this.mediaService.updateRendererConfig(config)
  }

  async startMic(): Promise<void> {
    await this.sessionService.getChatChannel()?.startMic()
  }

  async stopMic(): Promise<void> {
    this.sessionService.getChatChannel()?.stopMic()
  }

  snapshotStats(): Promise<StreamStats> {
    return this.statsService.snapshot()
  }

  subscribeStats(listener: (stats: StreamStats) => void): () => void {
    return this.emitter.on('stats.updated', listener)
  }

  close(): void {
    this.statsService.stop()
    this.sessionService.close()
  }

  reset(): void {
    this.sessionService.close()
    this.statsService.start()
  }

  setGamepadState(state: GamepadFrame): void {
    this.inputService.setGamepadState(state)
  }

  pressButton(button: LogicalButtonDto, durationMs: number): void {
    this.pressButtonStart(button)
    window.setTimeout(() => {
      this.pressButtonEnd(button)
    }, durationMs)
  }

  pressButtonStart(button: LogicalButtonDto): void {
    this.inputService.pressButtonStart(button)
  }

  pressButtonEnd(button: LogicalButtonDto): void {
    this.inputService.pressButtonEnd(button)
  }

  moveLeftStick(x: number, y: number): void {
    this.inputService.moveLeftStick(x, y)
  }

  moveRightStick(x: number, y: number): void {
    this.inputService.moveRightStick(x, y)
  }

  setAudioVolumeDirect(value: number): void {
    this.mediaService.setVolumeDirect(value)
  }

  getMicState(): { capturing: boolean, paused: boolean } {
    return this.mediaService.getMicState()
  }

  input(): PlayerInputController {
    return {
      updateConfig: config => this.updateInputConfig(config),
      setGamepadState: state => this.setGamepadState(state),
      pressButton: (button, durationMs) => this.pressButton(button, durationMs),
      pressButtonStart: button => this.pressButtonStart(button),
      pressButtonEnd: button => this.pressButtonEnd(button),
      moveLeftStick: (x, y) => this.moveLeftStick(x, y),
      moveRightStick: (x, y) => this.moveRightStick(x, y),
    }
  }

  audio(): PlayerAudioController {
    return {
      updateConfig: config => this.updateAudioConfig(config),
      setVolumeDirect: value => this.setAudioVolumeDirect(value),
      startMic: () => this.startMic(),
      stopMic: () => this.stopMic(),
      getMicState: () => this.getMicState(),
    }
  }

  stats(): PlayerStatsController {
    return {
      snapshot: () => this.snapshotStats(),
      subscribe: listener => this.subscribeStats(listener),
    }
  }

  private resolveContainer(): HTMLElement {
    if (typeof this.options.container === 'string') {
      const element = document.getElementById(this.options.container)
      if (!element) {
        throw new Error(`Container not found: ${this.options.container}`)
      }
      return element
    }
    return this.options.container
  }

  private createInputDriver(inputDriver?: PlayerInputDriver | null): InputDriverLike {
    if (inputDriver === null) {
      return {
        start: () => undefined,
        stop: () => undefined,
        requestStates: () => [],
      }
    }
    if (inputDriver !== undefined) {
      return inputDriver
    }
    return new GamepadDriver({
      onGamepadAdded: index => this.sessionService.getControlChannel()?.sendGamepadAdded(index),
      onGamepadRemoved: index =>
        this.sessionService.getControlChannel()?.sendGamepadRemoved(index),
      onFrame: frame => this.inputService.queueGamepadState(frame),
      getRuntimeConfig: () => this.options.input,
    })
  }

  private resolveInputDriverOption(inputDriver: unknown): PlayerInputDriver | null | undefined {
    if (inputDriver === null) {
      return null
    }

    return inputDriver !== undefined ? (inputDriver as PlayerInputDriver) : undefined
  }
}
