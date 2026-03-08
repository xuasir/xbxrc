import { app } from 'electron'
import { registerRpc } from '../../rpc/register'
import { getConfigService } from '../../mods/config'
import type { ShellLaunchSettings } from '../domain/types'
import { resolveStartupFlags } from './feat/startup-flags'
import { MainWindowManager } from './feat/window-manager'
import { SessionOrchestrator } from './feat/session-orchestrator'
import { ShellLifecycleManager } from './feat/lifecycle-manager'
import { ShellRpcController } from './feat/shell-rpc-controller'

interface ShellBootstrapDeps {
  preloadPath: string
  rendererHtmlPath: string
  linuxIcon?: string
  devRendererUrl?: string
}

export class ShellBootstrap {
  private readonly windowManager: MainWindowManager
  private readonly rpcController: ShellRpcController
  private readonly lifecycleManager: ShellLifecycleManager
  private started = false

  constructor(deps: ShellBootstrapDeps) {
    this.windowManager = new MainWindowManager({
      preloadPath: deps.preloadPath,
      rendererHtmlPath: deps.rendererHtmlPath,
      linuxIcon: deps.linuxIcon,
      devRendererUrl: deps.devRendererUrl
    })

    this.rpcController = new ShellRpcController({
      windowManager: this.windowManager
    })

    this.lifecycleManager = new ShellLifecycleManager({
      windowManager: this.windowManager,
      sessionOrchestrator: new SessionOrchestrator(),
      registerShellRpc: (rpcController) => {
        // 通过注入回调解耦 lifecycle 与 rpc 细节
        registerRpc({ rpcController })
      }
    })
  }

  start(): void {
    if (this.started) {
      return
    }
    this.started = true

    const launchSettings = this.getLaunchSettings()
    this.applyGpuSwitches(launchSettings.useVulkan)
    this.rpcController.setStartupFlags(resolveStartupFlags(process.argv, ''))

    this.lifecycleManager.bind({
      rpcController: this.rpcController,
      initialSettings: launchSettings,
      getLaunchSettings: () => this.getLaunchSettings(),
      getStartupFlags: () => this.rpcController.getStartupFlags()
    })
  }

  /**
   * 读取并归一化壳层启动配置，统一作为窗口与启动参数的输入。
   */
  private getLaunchSettings(): ShellLaunchSettings {
    const config = getConfigService().getByKeys([
      'fullscreen',
      'background_keepalive',
      'use_vulkan'
    ])

    return {
      fullscreen: config.fullscreen === true,
      backgroundKeepalive: config.background_keepalive === true,
      useVulkan: config.use_vulkan === true
    }
  }

  /**
   * 在 app ready 前注入 GPU/渲染相关开关，保持原有启动语义。
   */
  private applyGpuSwitches(useVulkan: boolean): void {
    if (useVulkan) {
      app.commandLine.appendSwitch('use-vulkan')
      app.commandLine.appendSwitch(
        'enable-features',
        'Vulkan,VulkanFromANGLE,DefaultANGLEVulkan,VaapiIgnoreDriverChecks,VaapiVideoDecoder,PlatformHEVCDecoderSupport,CanvasOopRasterization'
      )
      app.commandLine.appendSwitch('enable-gpu-rasterization')
      app.commandLine.appendSwitch('enable-oop-rasterization')
      app.commandLine.appendSwitch('enable-accelerated-video-decode')
      app.commandLine.appendSwitch('ozone-platform-hint', 'x11')
      app.commandLine.appendSwitch('ignore-gpu-blocklist')
      app.commandLine.appendSwitch('no-sandbox')
      app.commandLine.appendSwitch('enable-zero-copy')
      return
    }

    app.commandLine.appendSwitch('ignore-gpu-blacklist')
    app.commandLine.appendSwitch('enable-gpu-rasterization')
    app.commandLine.appendSwitch('enable-oop-rasterization')
    app.commandLine.appendSwitch('enable-accelerated-video-decode')
    app.commandLine.appendSwitch('ozone-platform-hint', 'x11')
  }
}
