export interface StreamDataChannelProfile {
  name: 'input' | 'control' | 'chat' | 'message'
  protocol: string
  ordered: boolean
}

// 串流协议固定网络合同，Rust sidecar 需要按同一套值镜像实现。
export const STREAM_DATA_CHANNEL_PROFILES: ReadonlyArray<StreamDataChannelProfile> = [
  { name: 'input', protocol: '1.0', ordered: true },
  { name: 'control', protocol: 'controlV1', ordered: true },
  { name: 'chat', protocol: 'chatV1', ordered: true },
  { name: 'message', protocol: 'messageV1', ordered: true }
]

export const STREAM_MESSAGE_HANDSHAKE = {
  version: 'messageV1',
  id: 'f9c5f412-0e69-4ede-8e62-92c7f5358c56',
  transactionId: '41f93d5a-900f-4d33-b7a1-2d4ca6747072',
  clientAppInstallId: 'c11ddb2e-c7e3-4f02-a62b-fd5448e0b851'
} as const

export const STREAM_CONTROL_PROFILE = {
  accessKey: '4BDB3609-C1F1-4195-9B37-FEFF45DA8B8E',
  keyframeIntervalMs: 5000,
  gamepadAddedDelayMs: 500
} as const

export const STREAM_INPUT_PROFILE = {
  initialMaxTouchpoints: 2
} as const

export const STREAM_DEFAULT_VIEWPORT = {
  width: 1920,
  height: 1080
} as const
