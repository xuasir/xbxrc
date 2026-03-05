import type { AppConfig } from './types'

export const DEFAULT_APP_CONFIG: AppConfig = {
  // 应用语言（en / zh / zht 等）
  locale: 'en',
  // 是否启用 MSAL 认证
  use_msal: false,
  // 启动时是否全屏
  fullscreen: false,
  // 串流分辨率（720 / 1080 / 1081）
  resolution: 720,
  // xHome 自动连接的主机 ID
  xhome_auto_connect_server_id: '',
  // xHome 码率模式（Auto / Custom）
  xhome_bitrate_mode: 'Auto',
  // xHome 目标码率（Mb/s）
  xhome_bitrate: 20,
  // xHome 是否启用 TURN fallback
  xhome_turn_fallback: false,
  // xCloud 码率模式（Auto / Custom）
  xcloud_bitrate_mode: 'Auto',
  // xCloud 目标码率（Mb/s）
  xcloud_bitrate: 20,
  // 音频码率模式（Auto / Custom）
  audio_bitrate_mode: 'Auto',
  // 音频目标码率（Mb/s）
  audio_bitrate: 20,
  // 是否启用音量控制
  enable_audio_control: false,
  // 游戏偏好语言（如 en-US）
  preferred_game_language: 'en-US',
  // 强制区域 IP
  force_region_ip: '',
  // 视频编码偏好
  codec: '',
  // 手柄轮询率（Hz）
  polling_rate: 250,
  // 是否启用手柄震动
  vibration: true,
  // 串流时自动开机
  power_on: false,
  // 视频显示格式
  video_format: '',
  // 是否优先 IPv6
  ipv6: false,
  // 性能面板展示样式
  performance_style: false,
  // 串流 runtime 模式
  stream_runtime_mode: 'webrtc-direct',
  // 自建服务器地址
  server_url: '',
  // 自建服务器用户名
  server_username: '',
  // 自建服务器凭证
  server_credential: '',
  // 后台保活
  background_keepalive: false,
  display_options: {
    // 画面锐化强度
    sharpness: 2,
    // 画面饱和度
    saturation: 100,
    // 画面对比度
    contrast: 100,
    // 画面亮度
    brightness: 100
  },
  // 是否启用 Vulkan 渲染路径
  use_vulkan: false,
  // 调试模式开关
  debug: false
}

export function getDefaultConfig(): AppConfig {
  // 冻结默认配置对象，防止运行时修改
  return Object.freeze(DEFAULT_APP_CONFIG)
}
