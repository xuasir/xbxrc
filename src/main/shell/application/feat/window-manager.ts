import { BrowserWindow, shell } from 'electron'

interface MainWindowManagerDeps {
  preloadPath: string
  rendererHtmlPath: string
  linuxIcon?: string
  devRendererUrl?: string
}

interface CreateWindowOptions {
  fullscreen: boolean
  backgroundKeepalive: boolean
}

export class MainWindowManager {
  private mainWindow: BrowserWindow | undefined
  private isQuitting = false

  // 以 1080p 作为桌面端默认设计基线，后续多分辨率适配都围绕该尺寸展开。
  private static readonly DEFAULT_WINDOW_WIDTH = 1920
  private static readonly DEFAULT_WINDOW_HEIGHT = 1080

  private readonly preloadPath: string
  private readonly rendererHtmlPath: string
  private readonly linuxIcon?: string
  private readonly devRendererUrl?: string

  constructor(deps: MainWindowManagerDeps) {
    this.preloadPath = deps.preloadPath
    this.rendererHtmlPath = deps.rendererHtmlPath
    this.linuxIcon = deps.linuxIcon
    this.devRendererUrl = deps.devRendererUrl
  }

  setQuitting(value: boolean): void {
    this.isQuitting = value
  }

  getWindow(): BrowserWindow | undefined {
    return this.mainWindow
  }

  isFullscreen(): boolean {
    if (this.mainWindow === undefined || this.mainWindow.isDestroyed()) {
      return false
    }
    return this.mainWindow.isFullScreen()
  }

  setFullscreen(fullscreen: boolean): boolean {
    if (this.mainWindow === undefined || this.mainWindow.isDestroyed()) {
      return false
    }
    this.mainWindow.setFullScreen(fullscreen)
    return this.mainWindow.isFullScreen()
  }

  toggleFullscreen(): boolean {
    if (this.mainWindow === undefined || this.mainWindow.isDestroyed()) {
      return false
    }
    const next = !this.mainWindow.isFullScreen()
    this.mainWindow.setFullScreen(next)
    return this.mainWindow.isFullScreen()
  }

  createOrShow(options: CreateWindowOptions): BrowserWindow {
    if (this.mainWindow !== undefined && !this.mainWindow.isDestroyed()) {
      this.mainWindow.show()
      return this.mainWindow
    }

    const window = new BrowserWindow({
      width: MainWindowManager.DEFAULT_WINDOW_WIDTH,
      height: MainWindowManager.DEFAULT_WINDOW_HEIGHT,
      show: false,
      autoHideMenuBar: true,
      backgroundColor: 'rgb(26, 27, 30)',
      fullscreen: options.fullscreen,
      ...(process.platform === 'linux' && this.linuxIcon ? { icon: this.linuxIcon } : {}),
      webPreferences: {
        preload: this.preloadPath,
        sandbox: false
      }
    })

    window.on('ready-to-show', () => {
      window.show()
    })

    window.webContents.setWindowOpenHandler((details) => {
      void shell.openExternal(details.url)
      return { action: 'deny' }
    })

    if (options.backgroundKeepalive) {
      window.webContents.setBackgroundThrottling(false)
    }

    window.on('close', (event) => {
      // macOS 维持“关闭窗口不退出”的常驻语义
      if (process.platform === 'darwin' && !this.isQuitting) {
        event.preventDefault()
        window.hide()
      }
    })

    window.on('closed', () => {
      this.mainWindow = undefined
    })

    if (this.devRendererUrl !== undefined && this.devRendererUrl !== '') {
      void window.loadURL(this.devRendererUrl)
    } else {
      void window.loadFile(this.rendererHtmlPath)
    }

    this.mainWindow = window
    return window
  }
}
