import { app } from 'electron'
import type { ShellRpcAdapter, StartupFlags } from '../../domain/types'
import { MainWindowManager } from './window-manager'

interface ShellRpcControllerDeps {
  windowManager: MainWindowManager
}

/**
 * 壳层 RPC 控制器
 * - 对外暴露 shell rpc 能力，内部委托具体子能力模块
 */
export class ShellRpcController implements ShellRpcAdapter {
  private readonly windowManager: MainWindowManager
  private startupFlags: StartupFlags = {
    fullscreen: false,
    autoConnect: ''
  }

  constructor(deps: ShellRpcControllerDeps) {
    this.windowManager = deps.windowManager
  }

  setStartupFlags(flags: StartupFlags): void {
    this.startupFlags = { ...flags }
  }

  getStartupFlags(): StartupFlags {
    return { ...this.startupFlags }
  }

  resetAutoConnect(): void {
    this.startupFlags.autoConnect = ''
  }

  isFullscreen(): boolean {
    return this.windowManager.isFullscreen()
  }

  toggleFullscreen(): boolean {
    return this.windowManager.toggleFullscreen()
  }

  enterFullscreen(): boolean {
    return this.windowManager.setFullscreen(true)
  }

  exitFullscreen(): boolean {
    return this.windowManager.setFullscreen(false)
  }

  quit(): boolean {
    setTimeout(() => {
      app.quit()
    }, 10)
    return true
  }

  restart(): boolean {
    setTimeout(() => {
      app.relaunch()
      app.exit(0)
    }, 10)
    return true
  }
}
