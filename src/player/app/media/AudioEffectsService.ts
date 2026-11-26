import type { AudioPlaybackFormat, AudioPlaybackObservation, AudioRuntimeConfig } from '../../domain/media'
import { resolveAudioPlaybackFormat } from '../../infra/webrtc/audio-sdp'

export class AudioEffectsService {
  private context?: AudioContext
  private source?: MediaStreamAudioSourceNode
  private gain?: GainNode
  private audio?: HTMLAudioElement
  private stream?: MediaStream
  private track?: MediaStreamTrack
  private config?: AudioRuntimeConfig
  private format?: AudioPlaybackFormat
  private outputChannelCount?: number
  private channelFallbackReason?: string
  private generation = 0
  private lifecycle?: AbortController
  private cleanups: Array<() => void> = []
  private contextCleanup?: () => void
  private operationTimers = new Set<ReturnType<typeof setTimeout>>()
  private flight?: Promise<void>
  private starting = false
  private playAccepted = false
  private waiting = false
  private errorKind?: string
  private recoveryAttempts = 0
  private recoveryReason?: string
  private lastRecoveryAt = Number.NEGATIVE_INFINITY
  private graphRebuilt = false
  private observationDigest = ''

  constructor(private readonly onObservation?: (observation: AudioPlaybackObservation) => void) {}

  snapshotResources() {
    return {
      audioContexts: Number(this.context !== undefined),
      audioNodes: Number(this.source !== undefined) + Number(this.gain !== undefined),
      audioListeners: this.cleanups.length + Number(this.contextCleanup !== undefined),
      audioOperationTimers: this.operationTimers.size,
      audioFormat: this.format ? { ...this.format } : undefined,
      audioOutputChannelCount: this.outputChannelCount,
      audioChannelExecution: this.context ? 'explicit-graph' : this.channelFallbackReason ? 'fallback-direct' : 'direct-browser',
      audioChannelFallbackReason: this.channelFallbackReason,
    }
  }

  snapshotObservation(): AudioPlaybackObservation {
    let state: AudioPlaybackObservation['state'] = 'idle'
    if (this.audio) {
      if (this.track?.readyState === 'ended')
        state = 'ended'
      else if (this.errorKind)
        state = this.errorKind === 'autoplayBlocked' ? 'blocked' : 'failed'
      else if (this.track?.muted)
        state = 'waiting'
      else if (this.context)
        state = this.context.state === 'running' ? 'ready' : this.starting ? 'starting' : this.context.state === 'closed' ? 'failed' : 'waiting'
      else if (this.starting)
        state = 'starting'
      else if (this.waiting)
        state = 'waiting'
      else if (this.playAccepted)
        state = this.audio.paused ? 'paused' : 'ready'
      else
        state = 'waiting'
    }
    return {
      generation: this.generation,
      observedAtMs: performance.now(),
      state,
      route: this.context ? 'web-audio' : 'direct',
      contextState: this.context?.state,
      trackState: this.track?.readyState,
      trackMuted: this.track?.muted,
      trackIdentifier: this.track?.id,
      elementPaused: this.audio?.paused,
      format: this.format ? { ...this.format } : undefined,
      outputChannelCount: this.outputChannelCount,
      channelFallbackReason: this.channelFallbackReason,
      intentionallySilent: (this.config?.volume ?? 1) <= 0,
      recoveryAttempts: this.recoveryAttempts,
      recoveryReason: this.recoveryReason,
      errorKind: this.errorKind,
    }
  }

  attach(stream: MediaStream, audio: HTMLAudioElement, config: AudioRuntimeConfig, format?: AudioPlaybackFormat, track?: MediaStreamTrack, graphFallbackReason?: string): void {
    this.destroy()
    this.audio = audio
    this.stream = stream
    this.track = track ?? stream.getAudioTracks?.()[0]
    this.config = { ...config }
    this.format = format ?? resolveAudioPlaybackFormat({ requestedChannelCount: config.requestedChannelCount ?? 2 })
    this.lifecycle = new AbortController()
    this.channelFallbackReason = graphFallbackReason
    this.errorKind = undefined
    this.playAccepted = this.waiting = this.graphRebuilt = false
    this.recoveryAttempts = 0
    this.recoveryReason = undefined
    this.lastRecoveryAt = Number.NEGATIVE_INFINITY
    audio.muted = false
    this.listen(audio, 'playing', () => {
      this.playAccepted = true
      this.waiting = false
      this.errorKind = undefined
      this.report()
    })
    this.listen(audio, 'waiting', () => {
      this.waiting = true
      this.report()
    })
    this.listen(audio, 'pause', () => this.report())
    this.listen(audio, 'error', () => {
      this.errorKind = 'elementPlaybackFailed'
      this.report()
    })
    this.listen(this.track, 'mute', () => this.report())
    this.listen(this.track, 'unmute', () => {
      this.refreshTrackFormat()
      void this.recover('trackUnmuted')
    })
    this.listen(this.track, 'ended', () => {
      this.lifecycle?.abort()
      this.releaseGraph()
      this.report()
    })
    if (typeof document !== 'undefined') {
      this.listen(document, 'visibilitychange', () => {
        if (!document.hidden)
          void this.recover('foreground')
      })
      const gesture = () => {
        const state = this.snapshotObservation().state
        if (this.flight && (state === 'starting' || state === 'waiting'))
          void this.resumeOutput('userGesture', this.generation).catch(() => {})
        else if (state === 'blocked' || state === 'waiting')
          void this.recover('userGesture')
      }
      this.listen(document, 'pointerdown', gesture)
      this.listen(document, 'keydown', gesture)
    }
    if (typeof navigator !== 'undefined')
      this.listen(navigator.mediaDevices, 'devicechange', () => { void this.recover('deviceChanged') })
    if (this.needsGraph() && graphFallbackReason === undefined) {
      try {
        this.createGraph()
      }
      catch {
        this.releaseGraph()
        this.channelFallbackReason = 'audioGraphUnavailable'
      }
    }
    this.updateVolume(config.volume)
    void this.startOutput('attach', true)
  }

  updateConfig(config: AudioRuntimeConfig): void {
    if ((this.config?.enableAudioControl !== config.enableAudioControl
      || (this.config?.requestedChannelCount ?? 2) !== (config.requestedChannelCount ?? 2)) && this.stream && this.audio) {
      const requested = config.requestedChannelCount ?? 2
      const format = this.format
        ? {
            ...this.format,
            requestedChannelCount: requested,
            effectiveChannelCount: requested === 1 ? 1 as const : this.format.sourceChannelCount ?? (this.format.senderStereo === false ? 1 as const : 2 as const),
          }
        : undefined
      this.attach(this.stream, this.audio, config, format, this.track)
      return
    }
    this.config = { ...config }
    this.updateVolume(config.volume)
  }

  updateFormat(format: AudioPlaybackFormat): void {
    const previous = this.format
    this.format = { ...format }
    if (this.config)
      this.config.requestedChannelCount = format.requestedChannelCount
    if (!this.audio || !this.stream || !this.config)
      return
    if (!this.needsGraph() && this.context) {
      // 路径切换重新建立代次，旧的 resume 会随生命周期撤销。
      this.attach(this.stream, this.audio, this.config, format, this.track)
      return
    }
    else if (!this.context && this.needsGraph() && (previous?.requestedChannelCount !== format.requestedChannelCount || !this.channelFallbackReason)) {
      this.attach(this.stream, this.audio, this.config, format, this.track)
      return
    }
    else {
      try {
        this.configureChannels()
      }
      catch {
        this.attach(this.stream, this.audio, this.config, format, this.track, 'audioChannelConfigurationFailed')
        return
      }
    }
    this.updateVolume(this.config.volume)
    this.report()
  }

  private needsGraph(): boolean {
    return this.config?.enableAudioControl === true || this.format?.requestedChannelCount === 1
  }

  private createGraph(): void {
    const context = new AudioContext({ latencyHint: 'interactive' })
    this.context = context
    this.source = context.createMediaStreamSource(this.stream!)
    this.gain = context.createGain()
    this.configureChannels()
    this.source.connect(this.gain).connect(context.destination)
    this.audio!.muted = true
    const changed = () => {
      if (this.context !== context)
        return
      this.report()
      if (context.state !== 'running' && !this.starting && (typeof document === 'undefined' || !document.hidden))
        void this.recover('contextStateChanged')
    }
    context.addEventListener?.('statechange', changed)
    this.contextCleanup = () => context.removeEventListener?.('statechange', changed)
  }

  private configureChannels(): void {
    if (!this.context || !this.gain || !this.format)
      return
    const channels = this.format.effectiveChannelCount
    this.gain.channelCount = channels
    this.gain.channelCountMode = 'explicit'
    this.gain.channelInterpretation = 'speakers'
    const maximum = this.context.destination.maxChannelCount
    const output = Number.isFinite(maximum) && maximum > 0 ? Math.min(channels, maximum) : channels
    this.context.destination.channelCount = output
    this.outputChannelCount = this.context.destination.channelCount
    this.channelFallbackReason = output < channels ? 'outputChannelLimit' : undefined
  }

  private refreshTrackFormat(): void {
    try {
      const channels = this.track?.getSettings?.().channelCount
      if (this.format && (channels === 1 || channels === 2))
        this.updateFormat({ ...this.format, sourceChannelCount: channels, source: 'trackSettings', effectiveChannelCount: this.format.requestedChannelCount === 1 ? 1 : channels })
    }
    catch {}
  }

  updateVolume(value: number): void {
    const volume = Number.isFinite(value) ? Math.max(0, value) : 1
    if (this.config)
      this.config.volume = volume
    if (this.gain)
      this.gain.gain.value = this.config?.enableAudioControl ? volume : Math.min(1, volume)
    if (this.audio)
      this.audio.volume = this.gain ? 1 : Math.min(1, volume)
    this.report()
  }

  recover(reason: string): Promise<void> {
    if (!this.audio || this.track?.readyState === 'ended' || (typeof document !== 'undefined' && document.hidden))
      return Promise.resolve()
    if (this.flight)
      return this.flight
    if (this.snapshotObservation().state === 'ready' && reason !== 'deviceChanged')
      return Promise.resolve()
    const now = performance.now()
    if (reason !== 'userGesture' && now - this.lastRecoveryAt < 2000)
      return Promise.resolve()
    this.lastRecoveryAt = now
    this.recoveryAttempts++
    return this.startOutput(reason, false)
  }

  private startOutput(reason: string, initial: boolean): Promise<void> {
    if (this.flight)
      return this.flight
    const generation = this.generation
    this.starting = true
    this.errorKind = undefined
    this.recoveryReason = reason
    this.report()
    const current = () => generation === this.generation && this.audio !== undefined && !this.lifecycle?.signal.aborted
    const run = this.resumeOutput(reason, generation).catch(async (error) => {
      if (!current())
        return
      if (this.context?.state === 'running' || (!this.context && this.playAccepted && this.audio?.paused === false))
        return
      if (this.context && !initial && !this.graphRebuilt && this.classifyError(error) !== 'autoplayBlocked') {
        this.graphRebuilt = true
        this.releaseGraph()
        try {
          this.createGraph()
          this.updateVolume(this.config?.volume ?? 1)
          await this.resumeOutput(reason, generation)
          return
        }
        catch (nextError) { error = nextError }
      }
      if (!current())
        return
      if (this.context) {
        this.releaseGraph()
        this.channelFallbackReason = 'audioGraphResumeFailed'
        this.audio!.muted = false
        this.updateVolume(this.config?.volume ?? 1)
        try {
          await this.playDirect(generation)
          return
        }
        catch (nextError) { error = nextError }
      }
      if (current())
        this.errorKind = this.classifyError(error)
    }).finally(() => {
      if (this.flight === run)
        this.flight = undefined
      if (current()) {
        this.starting = false
        this.report()
      }
    })
    this.flight = run
    return run
  }

  private async resumeOutput(reason: string, generation: number): Promise<void> {
    const context = this.context
    const output = context ?? this.audio
    const sink = output as typeof output & { setSinkId?: (id: string) => Promise<void> }
    if (reason === 'deviceChanged' && typeof sink?.setSinkId === 'function')
      await this.awaitOperation(sink.setSinkId(''))
    if (generation !== this.generation)
      return
    if (context) {
      await this.awaitOperation(context.resume())
      if (generation === this.generation && context.state !== 'running')
        throw new Error('audioContextStillSuspended')
    }
    else {
      await this.playDirect(generation)
    }
  }

  private async playDirect(generation: number): Promise<void> {
    if (typeof this.audio?.play !== 'function')
      throw new Error('audioPlaybackUnavailable')
    await this.awaitOperation(this.audio.play())
    if (generation === this.generation && !this.lifecycle?.signal.aborted)
      this.playAccepted = true
  }

  private awaitOperation(pending: Promise<unknown>): Promise<void> {
    const signal = this.lifecycle?.signal
    return new Promise<void>((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout> | undefined
      let finished = false
      let abortListener: (() => void) | undefined
      const finish = (error?: unknown) => {
        if (finished)
          return
        finished = true
        clearTimeout(timer)
        if (timer !== undefined)
          this.operationTimers.delete(timer)
        if (abortListener)
          signal?.removeEventListener('abort', abortListener)
        if (error !== undefined)
          reject(error)
        else resolve()
      }
      abortListener = () => finish(new Error('audioPlaybackSuperseded'))
      pending.then(() => finish(), error => finish(error))
      if (signal?.aborted) {
        abortListener()
        return
      }
      signal?.addEventListener('abort', abortListener, { once: true })
      timer = setTimeout(() => finish(new Error('audioOperationTimeout')), 2000)
      this.operationTimers.add(timer)
    })
  }

  private classifyError(error: unknown): string {
    const name = error instanceof Error ? error.name : ''
    const message = error instanceof Error ? error.message : ''
    return name === 'NotAllowedError' || /autoplay/i.test(message)
      ? 'autoplayBlocked'
      : message === 'audioOperationTimeout' ? 'operationTimeout' : 'playbackFailed'
  }

  private listen(target: EventTarget | undefined, event: string, listener: () => void): void {
    if (typeof target?.addEventListener !== 'function')
      return
    const generation = this.generation
    const callback = () => {
      if (generation === this.generation)
        listener()
    }
    target.addEventListener(event, callback)
    this.cleanups.push(() => target.removeEventListener(event, callback))
  }

  private report(): void {
    if (!this.onObservation)
      return
    const observation = this.snapshotObservation()
    const { observedAtMs: _, ...stable } = observation
    const digest = JSON.stringify(stable)
    if (digest !== this.observationDigest) {
      this.observationDigest = digest
      this.onObservation(observation)
    }
  }

  private releaseGraph(): void {
    this.contextCleanup?.()
    this.contextCleanup = undefined
    this.source?.disconnect()
    this.gain?.disconnect()
    if (this.context && this.context.state !== 'closed')
      void this.context.close().catch(() => {})
    this.source = undefined
    this.gain = undefined
    this.context = undefined
    this.outputChannelCount = undefined
  }

  destroy(): void {
    const hadAudio = this.audio !== undefined
    this.generation++
    this.lifecycle?.abort()
    this.lifecycle = undefined
    this.flight = undefined
    this.cleanups.splice(0).forEach(cleanup => cleanup())
    this.releaseGraph()
    if (this.audio)
      this.audio.muted = false
    this.audio = undefined
    this.stream = undefined
    this.track = undefined
    this.format = undefined
    this.starting = false
    if (hadAudio)
      this.report()
  }
}
