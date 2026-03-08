export interface FpsStats {
  video: number
  input: number
  metadata: number
}

export interface InputPacketStats {
  packetBytes: number
  metadataFrames: number
  gamepadFrames: number
  pointerFrames: number
  mouseFrames: number
  keyboardFrames: number
}

export interface NetworkStats {
  roundTripTime: string
  packetLoss: string
  frameLoss: string
  jitter: string
  bitrate: string
}

export interface DecodeStats {
  fps: number
  decode: string
  resolution: string
}
