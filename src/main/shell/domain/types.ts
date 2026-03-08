export interface StartupFlags {
  fullscreen: boolean
  autoConnect: string
}

export interface ShellLaunchSettings {
  fullscreen: boolean
  backgroundKeepalive: boolean
  useVulkan: boolean
}

/**
 * shell 对 rpc 暴露的能力边界
 * - 供 rpc 层通过依赖注入调用，避免直接耦合壳层实现细节
 */
export interface ShellRpcAdapter {
  isFullscreen(): boolean
  toggleFullscreen(): boolean
  enterFullscreen(): boolean
  exitFullscreen(): boolean
  getStartupFlags(): StartupFlags
  resetAutoConnect(): void
  quit(): boolean
  restart(): boolean
}
