import type { RendererRuntimeConfig } from '../../domain/media'
import type { RendererResources } from './render-observation'
import type { RenderSurfaceObservation } from './render-surface-observation'
import type { WebGLFrameProcessor } from './webgl-frame-processor'
import { observeRenderSurface } from './render-surface-observation'
import { RenderWorkerHost } from './render-worker-host'
import { captureVideoElementToCanvas } from './webgl-frame-capture'
import { resolveShaderPreset } from './webgl-frame-processor'
import { createMainWebGLProcessor } from './webgl-main-surface'

export interface VideoRenderer {
  readonly kind: 'video' | 'webgl2' | 'webgl2_sr'
  attach: (video: HTMLVideoElement) => Promise<void> | void
  update: (config: Partial<RendererRuntimeConfig>) => void
  destroy: () => void
  captureRenderedFrame: (signal?: AbortSignal) => Promise<HTMLCanvasElement | null>
  snapshotResources: () => RendererResources | undefined
  snapshotDiagnostics?: () => RenderSurfaceObservation | undefined
}

let filterSequence = 0

export class NativeVideoRenderer implements VideoRenderer {
  readonly kind = 'video'
  private readonly filterId = `xbx-filter-${++filterSequence}`
  private config: RendererRuntimeConfig
  private styleElement: HTMLStyleElement | null = null
  private videoElement: HTMLVideoElement | null = null

  constructor(config: RendererRuntimeConfig) {
    this.config = config
  }

  snapshotResources(): RendererResources {
    return { outputWidth: this.videoElement?.videoWidth, outputHeight: this.videoElement?.videoHeight, gpuObjects: 0, frameRequests: 0, animationCallbacks: 0, captureWaiters: 0, domNodes: Number(this.styleElement !== null) }
  }

  snapshotDiagnostics(): RenderSurfaceObservation | undefined {
    return this.videoElement ? observeRenderSurface(this.videoElement) : undefined
  }

  attach(video: HTMLVideoElement): void {
    this.videoElement = video
    video.dataset.renderFilter = this.filterId
    this.ensureFilterNodes()
    video.dataset.renderPipeline = 'video'
    this.refreshVideoFilterStyle()
  }

  update(config: Partial<RendererRuntimeConfig>): void {
    this.config = { ...this.config, ...config }
    this.refreshVideoFilterStyle()
  }

  async captureRenderedFrame(signal?: AbortSignal): Promise<HTMLCanvasElement | null> {
    const video = this.videoElement
    if (video === null || signal?.aborted) {
      return null
    }
    return captureVideoElementToCanvas(video)
  }

  destroy(): void {
    if (this.videoElement?.dataset.renderFilter === this.filterId)
      delete this.videoElement.dataset.renderFilter
    this.videoElement = null
    this.styleElement?.remove()
    this.styleElement = null
  }

  private ensureFilterNodes(): void {
    if (!this.styleElement) {
      this.styleElement = document.createElement('style')
      this.styleElement.id = `${this.filterId}-css`
      document.documentElement.appendChild(this.styleElement)
    }
  }

  private refreshVideoFilterStyle(): void {
    if (!this.styleElement) {
      return
    }
    const filters: string[] = []
    if (this.config.saturation !== 100) {
      filters.push(`saturate(${this.config.saturation}%)`)
    }
    if (this.config.contrast !== 100) {
      filters.push(`contrast(${this.config.contrast}%)`)
    }
    if (this.config.brightness !== 100) {
      filters.push(`brightness(${this.config.brightness}%)`)
    }
    this.styleElement.textContent = filters.length > 0 ? `video[data-render-filter="${this.filterId}"] { filter: ${filters.join(' ')} !important; }` : ''
  }
}

export class WebGL2VideoRenderer implements VideoRenderer {
  get kind(): 'webgl2' | 'webgl2_sr' { return this.config.superResolutionEnabled ? 'webgl2_sr' : 'webgl2' }
  private player: WebGLFrameProcessor<HTMLCanvasElement> | null = null
  private workerHost: RenderWorkerHost | null = null
  private executionFallbackReason?: string
  private config: RendererRuntimeConfig

  constructor(config: RendererRuntimeConfig) {
    this.config = config
  }

  snapshotResources(): RendererResources | undefined {
    if (this.workerHost)
      return this.workerHost.snapshotResources()
    const resources = this.player?.snapshotResources()
    return resources ? { ...resources, execution: { mode: 'main', state: 'ready', capabilityReason: this.executionFallbackReason } } : undefined
  }

  snapshotDiagnostics(): RenderSurfaceObservation | undefined {
    return this.workerHost?.snapshotDiagnostics() ?? this.player?.snapshotDiagnostics()
  }

  async attach(video: HTMLVideoElement): Promise<void> {
    this.destroy()
    this.executionFallbackReason = undefined
    if (RenderWorkerHost.canAttempt(this.config)) {
      const host = new RenderWorkerHost(video, this.config)
      this.workerHost = host
      try {
        await host.attach()
        if (this.workerHost !== host)
          return
        video.dataset.renderPipeline = this.kind === 'webgl2_sr' ? 'webgl2-sr' : 'webgl2'
        return
      }
      catch (error) {
        if (this.workerHost !== host)
          return
        this.executionFallbackReason = error instanceof Error ? error.message : 'offscreenUnsupported'
        host.destroy()
        this.workerHost = null
      }
    }
    this.player = createMainWebGLProcessor(video)
    this.player.applyRendererPresentationConfig(this.config)
    this.player.setDisplayFormat(this.config.format)
    const preset = resolveShaderPreset(this.config)
    this.player.updateOptions({
      targetFps: this.config.targetFps,
      sharpness: preset.sharpenFactor,
      brightness: this.config.brightness,
      contrast: this.config.contrast,
      saturation: this.config.saturation,
      processingMode: preset.processingMode,
      processing: preset.processing,
    })
    const player = this.player
    await player.init()
    if (this.player !== player)
      return
    video.dataset.renderPipeline = this.kind === 'webgl2_sr' ? 'webgl2-sr' : 'webgl2'
    this.config.rendererExecutionNotifier?.('rendererExecutionObserved', this.snapshotResources()!)
  }

  update(config: Partial<RendererRuntimeConfig>): void {
    this.config = { ...this.config, ...config }
    this.workerHost?.update(config)
    this.player?.applyRendererPresentationConfig(this.config)
    this.player?.setDisplayFormat(this.config.format)
    const preset = resolveShaderPreset(this.config)
    this.player?.updateOptions({
      targetFps: this.config.targetFps,
      sharpness: preset.sharpenFactor,
      brightness: this.config.brightness,
      contrast: this.config.contrast,
      saturation: this.config.saturation,
      processing: preset.processing,
      processingMode: preset.processingMode,
    }, true)
  }

  destroy(): void {
    this.workerHost?.destroy()
    this.workerHost = null
    this.player?.destroy()
    this.player = null
  }

  async captureRenderedFrame(signal?: AbortSignal): Promise<HTMLCanvasElement | null> {
    if (this.workerHost)
      return this.workerHost.captureRenderedFrame(signal)
    if (this.player === null) {
      return null
    }
    return await this.player.captureRenderedFrame(signal)
  }
}

/** 既有 SR 装配入口复用同一引擎与时钟，保留诊断身份。 */
export class SuperResolutionWebGL2Renderer extends WebGL2VideoRenderer {
  constructor(config: RendererRuntimeConfig) { super({ ...config, superResolutionEnabled: true }) }
}
