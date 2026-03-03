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
  // 是否启用音频驱动震动
  enable_audio_rumble: false,
  // 音频驱动震动阈值
  audio_rumble_threshold: 0.15,
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
  // 手柄震动模式
  vibration_mode: 'Native',
  // 手柄内核模式
  gamepad_kernal: 'Native',
  // 是否启用手柄混合输入
  gamepad_mix: false,
  // 指定手柄索引（-1 自动）
  gamepad_index: -1,
  // 摇杆死区
  dead_zone: 0.1,
  // 摇杆边缘补偿
  edge_compensation: 0,
  // 扳机震动方向
  force_trigger_rumble: '',
  // 串流时自动开机
  power_on: false,
  // 视频显示格式
  video_format: '',
  // 虚拟手柄透明度
  virtual_gamepad_opacity: 0.6,
  // 自定义手柄映射
  gamepad_maping: null,
  // 是否优先 IPv6
  ipv6: false,
  // 是否启用原生键鼠
  enable_native_mouse_keyboard: false,
  // 鼠标灵敏度
  mouse_sensitive: 0.5,
  // 性能面板展示样式
  performance_style: false,
  // 自建服务器地址
  server_url: '',
  // 自建服务器用户名
  server_username: '',
  // 自建服务器凭证
  server_credential: '',
  // 后台保活
  background_keepalive: false,
  // 键鼠映射表
  input_mousekeyboard_maping: {
    ArrowLeft: 'DPadLeft',
    ArrowUp: 'DPadUp',
    ArrowRight: 'DPadRight',
    ArrowDown: 'DPadDown',
    Enter: 'A',
    k: 'A',
    Backspace: 'B',
    l: 'B',
    j: 'X',
    i: 'Y',
    '2': 'LeftShoulder',
    '3': 'RightShoulder',
    '1': 'LeftTrigger',
    '4': 'RightTrigger',
    '5': 'LeftThumb',
    '6': 'RightThumb',
    a: 'LeftThumbXAxisPlus',
    d: 'LeftThumbXAxisMinus',
    w: 'LeftThumbYAxisPlus',
    s: 'LeftThumbYAxisMinus',
    f: 'RightThumbXAxisPlus',
    h: 'RightThumbXAxisMinus',
    t: 'RightThumbYAxisPlus',
    g: 'RightThumbYAxisMinus',
    v: 'View',
    m: 'Menu',
    n: 'Nexus'
  },
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
