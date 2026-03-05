import { z } from 'zod'
import { getDefaultConfig } from './defaults'
import { APP_CONFIG_KEYS, type AppConfig, type DisplayOptions } from './types'

type UnknownRecord = Record<string, unknown>

const VALID_RESOLUTIONS = [720, 1080, 1081] as const

function isRecord(value: unknown): value is UnknownRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function toFiniteNumber(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value)) {
    return value
  }
  if (typeof value === 'string' && value.trim() !== '') {
    const parsed = Number(value)
    if (Number.isFinite(parsed)) {
      return parsed
    }
  }
  return undefined
}

function createBooleanSchema(fallback: boolean): z.ZodType<boolean> {
  return z
    .preprocess((value) => (typeof value === 'boolean' ? value : undefined), z.boolean())
    .default(fallback)
    .catch(fallback)
}

function createStringSchema(
  fallback: string,
  options?: { trim?: boolean; fallbackOnEmpty?: boolean }
): z.ZodType<string> {
  const trim = options?.trim ?? true
  const fallbackOnEmpty = options?.fallbackOnEmpty ?? false

  return z
    .preprocess((value) => {
      if (typeof value !== 'string') {
        return undefined
      }
      const next = trim ? value.trim() : value
      if (fallbackOnEmpty && next === '') {
        return undefined
      }
      return next
    }, z.string())
    .default(fallback)
    .catch(fallback)
}

function createNumberSchema(
  fallback: number,
  options?: {
    min?: number
    max?: number
    integer?: boolean
    allowed?: readonly number[]
  }
): z.ZodType<number> {
  return z
    .preprocess((value) => toFiniteNumber(value), z.number())
    .transform((value) => {
      let next = value
      if (options?.integer) {
        next = Math.round(next)
      }
      if (options?.min !== undefined && next < options.min) {
        next = options.min
      }
      if (options?.max !== undefined && next > options.max) {
        next = options.max
      }
      return next
    })
    .refine(
      (value) => (options?.allowed === undefined ? true : options.allowed.includes(value)),
      'Invalid number option'
    )
    .default(fallback)
    .catch(fallback)
}

function createDisplayOptionsSchema(fallback: DisplayOptions): z.ZodType<DisplayOptions> {
  return z.object({
    // 画面锐化强度，范围 [0, 10]
    sharpness: createNumberSchema(fallback.sharpness, { min: 0, max: 10 }),
    // 画面饱和度，范围 [0, 200]
    saturation: createNumberSchema(fallback.saturation, { min: 0, max: 200 }),
    // 画面对比度，范围 [0, 200]
    contrast: createNumberSchema(fallback.contrast, { min: 0, max: 200 }),
    // 画面亮度，范围 [0, 200]
    brightness: createNumberSchema(fallback.brightness, { min: 0, max: 200 })
  })
}

function createAppConfigSchema(fallback: AppConfig): z.ZodType<AppConfig> {
  return z.object({
    // 应用语言（例如 en / zh / zht）
    locale: createStringSchema(fallback.locale, { fallbackOnEmpty: true }),
    // 是否启用 MSAL 认证链路
    use_msal: createBooleanSchema(fallback.use_msal),
    // 启动时是否全屏
    fullscreen: createBooleanSchema(fallback.fullscreen),
    // 串流分辨率（720 / 1080 / 1081）
    resolution: createNumberSchema(fallback.resolution, {
      integer: true,
      allowed: VALID_RESOLUTIONS
    }),
    // xHome 自动连接主机 ID
    xhome_auto_connect_server_id: createStringSchema(fallback.xhome_auto_connect_server_id),
    // xHome 码率模式（Auto / Custom）
    xhome_bitrate_mode: z.enum(['Auto', 'Custom']).default(fallback.xhome_bitrate_mode),
    // xHome 目标码率（Mb/s）
    xhome_bitrate: createNumberSchema(fallback.xhome_bitrate, { min: 0, max: 200 }),
    // xHome 是否启用 TURN fallback
    xhome_turn_fallback: createBooleanSchema(fallback.xhome_turn_fallback),
    // xCloud 码率模式（Auto / Custom）
    xcloud_bitrate_mode: z.enum(['Auto', 'Custom']).default(fallback.xcloud_bitrate_mode),
    // xCloud 目标码率（Mb/s）
    xcloud_bitrate: createNumberSchema(fallback.xcloud_bitrate, { min: 0, max: 200 }),
    // 音频码率模式（Auto / Custom）
    audio_bitrate_mode: z.enum(['Auto', 'Custom']).default(fallback.audio_bitrate_mode),
    // 音频目标码率（Mb/s）
    audio_bitrate: createNumberSchema(fallback.audio_bitrate, { min: 0, max: 200 }),
    // 是否启用音量控制
    enable_audio_control: createBooleanSchema(fallback.enable_audio_control),
    // 游戏偏好语言（例如 en-US）
    preferred_game_language: createStringSchema(fallback.preferred_game_language),
    // 指定区域 IP（用于网络地区控制）
    force_region_ip: createStringSchema(fallback.force_region_ip),
    // 视频编码偏好
    codec: createStringSchema(fallback.codec),
    // 手柄轮询率（Hz）
    polling_rate: createNumberSchema(fallback.polling_rate, { min: 1, max: 1000 }),
    // 是否启用手柄震动
    vibration: createBooleanSchema(fallback.vibration),
    // 串流时是否自动唤醒主机
    power_on: createBooleanSchema(fallback.power_on),
    // 视频显示模式（拉伸/缩放/比例）
    video_format: createStringSchema(fallback.video_format),
    // 是否优先 IPv6 候选
    ipv6: createBooleanSchema(fallback.ipv6),
    // 性能面板展示样式
    performance_style: createBooleanSchema(fallback.performance_style),
    // 串流 runtime 模式
    stream_runtime_mode: z
      .enum(['webrtc-direct', 'rust-owned'])
      .default(fallback.stream_runtime_mode),
    // 自建服务器地址
    server_url: createStringSchema(fallback.server_url),
    // 自建服务器用户名
    server_username: createStringSchema(fallback.server_username),
    // 自建服务器凭证
    server_credential: createStringSchema(fallback.server_credential),
    // 后台保活
    background_keepalive: createBooleanSchema(fallback.background_keepalive),
    // 画面增强参数
    display_options: z
      .preprocess(
        (value) => (isRecord(value) ? value : {}),
        createDisplayOptionsSchema(fallback.display_options)
      )
      .default({ ...fallback.display_options }),
    // 是否启用 Vulkan
    use_vulkan: createBooleanSchema(fallback.use_vulkan),
    // 是否开启调试模式
    debug: createBooleanSchema(fallback.debug)
  })
}

export function pickConfigPatch(value: unknown): Partial<AppConfig> {
  if (!isRecord(value)) {
    return {}
  }

  const patch: Partial<AppConfig> = {}
  const typedPatch = patch as Record<keyof AppConfig, AppConfig[keyof AppConfig]>
  for (const key of APP_CONFIG_KEYS) {
    if (Object.prototype.hasOwnProperty.call(value, key)) {
      typedPatch[key] = value[key] as AppConfig[typeof key]
    }
  }
  return patch
}

export function parseAppConfig(value: unknown, fallback = getDefaultConfig()): AppConfig {
  const schema = createAppConfigSchema(fallback)
  const source = isRecord(value) ? value : {}
  return schema.parse(source)
}
