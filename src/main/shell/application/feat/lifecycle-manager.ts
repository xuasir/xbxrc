import { app, powerSaveBlocker } from 'electron'
import { electronApp, optimizer } from '@electron-toolkit/utils'
import type { ShellLaunchSettings, ShellRpcAdapter, StartupFlags } from '../../domain/types'
import { MainWindowManager } from './window-manager'
import { SessionOrchestrator } from './session-orchestrator'
import { registerAuthSessionReadyBridge } from './auth-session-ready-bridge'

interface ShellLifecycleManagerDeps {
  windowManager: MainWindowManager
  sessionOrchestrator: SessionOrchestrator
  registerShellRpc: (rpcController: ShellRpcAdapter) => void
}

interface BindOptions {
  rpcController: ShellRpcAdapter
  initialSettings: ShellLaunchSettings
  getLaunchSettings: () => ShellLaunchSettings
  getStartupFlags: () => StartupFlags
}

/**
 * 壳层生命周期管理器
 * - 绑定并维护 app 事件，不承载具体业务实现
 */
export class ShellLifecycleManager {
  private readonly windowManager: MainWindowManager
  private readonly sessionOrchestrator: SessionOrchestrator
  private readonly registerShellRpc: (rpcController: ShellRpcAdapter) => void
  private powerSaveBlockerId: number | undefined

  constructor(deps: ShellLifecycleManagerDeps) {
    this.windowManager = deps.windowManager
    this.sessionOrchestrator = deps.sessionOrchestrator
    this.registerShellRpc = deps.registerShellRpc
  }

  bind(options: BindOptions): void {
    app.whenReady().then(() => {
      electronApp.setAppUserModelId('com.electron')

      app.on('browser-window-created', (_, window) => {
        optimizer.watchWindowShortcuts(window)
      })

      this.registerShellRpc(options.rpcController)
      this.createOrShowMainWindow(options.initialSettings, options.getStartupFlags())
      registerAuthSessionReadyBridge()
      this.sessionOrchestrator.onAppReady()
      this.startPreventDisplaySleep()

      app.on('activate', () => {
        const latestSettings = options.getLaunchSettings()
        this.createOrShowMainWindow(latestSettings, options.getStartupFlags())
      })
    })

    app.on('window-all-closed', () => {
      if (process.platform !== 'darwin') {
        app.quit()
      }
    })

    app.on('before-quit', () => {
      this.windowManager.setQuitting(true)
      this.stopPreventDisplaySleep()
    })
  }

  private startPreventDisplaySleep(): void {
    if (
      this.powerSaveBlockerId !== undefined &&
      powerSaveBlocker.isStarted(this.powerSaveBlockerId)
    ) {
      return
    }
    this.powerSaveBlockerId = powerSaveBlocker.start('prevent-display-sleep')
  }

  private stopPreventDisplaySleep(): void {
    if (this.powerSaveBlockerId === undefined) {
      return
    }
    if (powerSaveBlocker.isStarted(this.powerSaveBlockerId)) {
      powerSaveBlocker.stop(this.powerSaveBlockerId)
    }
    this.powerSaveBlockerId = undefined
  }

  private createOrShowMainWindow(settings: ShellLaunchSettings, startupFlags: StartupFlags): void {
    this.windowManager.createOrShow({
      fullscreen: settings.fullscreen || startupFlags.fullscreen,
      backgroundKeepalive: settings.backgroundKeepalive
    })
  }
}
