export type VideoFit = 'Contain' | 'Stretch' | 'Zoom' | string

export interface AudioRuntimeConfig {
  volume: number
  enableAudioControl: boolean
}

export interface RendererRuntimeConfig {
  enabled: boolean
  sharpness: number
  mode: 'native' | 'webgl2'
  format: VideoFit
}

export interface StreamStats {
  resolution: string
  rtt: string
  fps: number
  pl: string
  fl: string
  jit: string
  br: string
  decode: string
}
