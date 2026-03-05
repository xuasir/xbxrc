import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import fs from 'node:fs'
import readline from 'node:readline'
import { createRequire } from 'node:module'
import path from 'node:path'
import process from 'node:process'

interface XbxEngineNativeAddonBinding {
  setRuntimeConfigJson?(configJson: string): void
  sendIncomingMessageJson(messageJson: string): void
  drainOutgoingMessagesJson(): string
  snapshotStatsJson(): string
  getLastRuntimeEventJson(): string
  shutdown(): void
}

interface XbxEngineNativeAddonModule {
  XbxEngineNativeBinding: new () => XbxEngineNativeAddonBinding
}

export interface XbxEngineNativeBindingListeners {
  onMessage(message: unknown): void
  onError(error: unknown): void
}

export interface XbxEngineNativeBinding {
  start(listeners: XbxEngineNativeBindingListeners): Promise<void>
  send(message: unknown): Promise<void>
  snapshotStats<TStats>(): Promise<TStats>
  getLastRuntimeEvent<TEvent>(): Promise<TEvent | null>
  shutdown(): Promise<void>
}

const NATIVE_POLL_INTERVAL_MS = 1000 / 60
const XBXENGINE_BINDING_MODE_ENV = 'XBXENGINE_BINDING'
const LEGACY_STREAMSIDECAR_BINDING_MODE_ENV = 'STREAMSIDECAR_BINDING'
const XBXENGINE_APP_PATH_ENV = 'XBXENGINE_APP_PATH'
const LEGACY_STREAMSIDECAR_APP_PATH_ENV = 'STREAMSIDECAR_APP_PATH'
const XBXENGINE_RUNTIME_CONFIG_PATH_ENV = 'XBXENGINE_RUNTIME_CONFIG_PATH'
const XBXENGINE_FORCE_REMB_KBPS_ENV = 'XBXENGINE_FORCE_REMB_KBPS'
const XBXENGINE_ADAPTIVE_REMB_ENABLED_ENV = 'XBXENGINE_ADAPTIVE_REMB_ENABLED'
const XBXENGINE_NACK_WINDOW_MS_ENV = 'XBXENGINE_NACK_WINDOW_MS'
const XBXENGINE_NACK_RETRY_INTERVAL_MS_ENV = 'XBXENGINE_NACK_RETRY_INTERVAL_MS'
const XBXENGINE_NACK_MAX_RETRY_COUNT_ENV = 'XBXENGINE_NACK_MAX_RETRY_COUNT'
const XBXENGINE_RTT_DIAGNOSTICS_ENABLED_ENV = 'XBXENGINE_RTT_DIAGNOSTICS_ENABLED'
const XBXENGINE_RTT_DIAGNOSTICS_LOG_INTERVAL_MS_ENV = 'XBXENGINE_RTT_DIAGNOSTICS_LOG_INTERVAL_MS'
const requireNativeModule = createRequire(import.meta.url)

/**
 * xbxEngine binding 只负责消息泵与宿主选择。
 * 当前主路径固定优先 `xbxengine-api` N-API；原生窗口验证时才显式切到 stdio 子进程宿主。
 */
export function createDefaultXbxEngineNativeBinding(): XbxEngineNativeBinding | null {
  const preferredMode = (
    process.env[XBXENGINE_BINDING_MODE_ENV] ??
    process.env[LEGACY_STREAMSIDECAR_BINDING_MODE_ENV] ??
    ''
  )
    .trim()
    .toLowerCase()

  if (preferredMode === 'stdio') {
    const stdioBinding = createStdioXbxEngineBinding()
    if (stdioBinding !== null) {
      return stdioBinding
    }
    console.warn(
      '[main][streaming] stdio XbxEngine binding requested but app binary was not found; falling back to N-API'
    )
  }

  const nativeModule = loadXbxEngineNativeAddon()
  if (nativeModule !== null) {
    return new NapiXbxEngineNativeBinding(nativeModule)
  }

  const stdioBinding = createStdioXbxEngineBinding()
  if (stdioBinding !== null) {
    return stdioBinding
  }

  return null
}

class NapiXbxEngineNativeBinding implements XbxEngineNativeBinding {
  private readonly addonBinding: XbxEngineNativeAddonBinding
  private pollTimer: NodeJS.Timeout | null = null

  constructor(nativeModule: XbxEngineNativeAddonModule) {
    this.addonBinding = new nativeModule.XbxEngineNativeBinding()
  }

  async start(listeners: XbxEngineNativeBindingListeners): Promise<void> {
    if (this.pollTimer !== null) {
      return
    }
    this.applyRuntimeConfig()

    const pumpMessages = (): void => {
      try {
        const messages = this.readJson<unknown[]>(this.addonBinding.drainOutgoingMessagesJson())
        messages.forEach((message) => {
          listeners.onMessage(message)
        })
      } catch (error) {
        listeners.onError(error)
      }
    }

    pumpMessages()
    this.pollTimer = setInterval(pumpMessages, NATIVE_POLL_INTERVAL_MS)
  }

  async send(message: unknown): Promise<void> {
    this.addonBinding.sendIncomingMessageJson(JSON.stringify(message))
  }

  async snapshotStats<TStats>(): Promise<TStats> {
    return this.readJson<TStats>(this.addonBinding.snapshotStatsJson())
  }

  async getLastRuntimeEvent<TEvent>(): Promise<TEvent | null> {
    return this.readJson<TEvent | null>(this.addonBinding.getLastRuntimeEventJson())
  }

  async shutdown(): Promise<void> {
    if (this.pollTimer !== null) {
      clearInterval(this.pollTimer)
      this.pollTimer = null
    }
    this.addonBinding.shutdown()
  }

  private readJson<T>(raw: string): T {
    return JSON.parse(raw) as T
  }

  private applyRuntimeConfig(): void {
    if (typeof this.addonBinding.setRuntimeConfigJson !== 'function') {
      return
    }
    const runtimeConfig = resolveXbxEngineRuntimeConfig()
    const webrtcConfig = isRecord(runtimeConfig.webrtc) ? runtimeConfig.webrtc : {}
    // 输出生效配置快照，方便排查误配置导致的码率异常。
    console.info('[main][streaming] xbxengine runtime config', {
      forcedRembKbps: webrtcConfig.forcedRembKbps ?? null,
      adaptiveRembEnabled: webrtcConfig.adaptiveRembEnabled ?? null,
      videoPipeline: webrtcConfig.videoPipeline ?? null,
      rttDiagnostics: webrtcConfig.rttDiagnostics ?? null
    })
    this.addonBinding.setRuntimeConfigJson(JSON.stringify(runtimeConfig))
  }
}

class StdioXbxEngineBinding implements XbxEngineNativeBinding {
  private readonly command: string
  private readonly args: string[]
  private childProcess: ChildProcessWithoutNullStreams | null = null
  private stdoutReader: readline.Interface | null = null
  private stderrReader: readline.Interface | null = null
  private listeners: XbxEngineNativeBindingListeners | null = null
  private shuttingDown = false
  private lastRuntimeEvent: unknown | null = null

  constructor(command: string, args: string[]) {
    this.command = command
    this.args = args
  }

  async start(listeners: XbxEngineNativeBindingListeners): Promise<void> {
    if (this.childProcess !== null) {
      return
    }

    this.listeners = listeners
    this.shuttingDown = false
    const childProcess = spawn(this.command, this.args, {
      cwd: process.cwd(),
      env: process.env,
      stdio: ['pipe', 'pipe', 'pipe']
    })
    this.childProcess = childProcess

    this.stdoutReader = readline.createInterface({
      input: childProcess.stdout,
      crlfDelay: Infinity
    })
    this.stdoutReader.on('line', (line) => {
      this.handleStdoutLine(line)
    })

    this.stderrReader = readline.createInterface({
      input: childProcess.stderr,
      crlfDelay: Infinity
    })
    this.stderrReader.on('line', (line) => {
      if (line.trim().length === 0) {
        return
      }
      // 原生侧 stderr 日志默认降噪，仅保留明显错误线索。
      if (isXbxEngineStderrImportant(line)) {
        console.warn('[main][streaming][xbxengine-app]', line)
      }
    })

    childProcess.on('error', (error) => {
      this.forwardError(error)
    })
    childProcess.on('exit', (code, signal) => {
      this.disposeReaders()
      this.childProcess = null
      if (this.shuttingDown) {
        return
      }
      const exitReason =
        signal !== null ? `signal:${signal}` : `code:${code === null ? 'unknown' : String(code)}`
      this.forwardError(new Error(`xbxengineAppExited:${exitReason}`))
    })
  }

  async send(message: unknown): Promise<void> {
    const childProcess = this.childProcess
    if (childProcess === null || childProcess.stdin.destroyed) {
      throw new Error('xbxengineAppProcessUnavailable')
    }

    await new Promise<void>((resolve, reject) => {
      childProcess.stdin.write(`${JSON.stringify(message)}\n`, (error) => {
        if (error) {
          reject(error)
          return
        }
        resolve()
      })
    })
  }

  async snapshotStats<TStats>(): Promise<TStats> {
    throw new Error('xbxengineSnapshotUnavailableInStdioBinding')
  }

  async getLastRuntimeEvent<TEvent>(): Promise<TEvent | null> {
    return (this.lastRuntimeEvent as TEvent | null) ?? null
  }

  async shutdown(): Promise<void> {
    this.shuttingDown = true
    const childProcess = this.childProcess
    this.disposeReaders()
    this.childProcess = null
    if (childProcess === null) {
      return
    }

    await new Promise<void>((resolve) => {
      let settled = false
      const finish = (): void => {
        if (settled) {
          return
        }
        settled = true
        resolve()
      }

      childProcess.once('exit', () => {
        finish()
      })

      childProcess.stdin.end(() => {
        childProcess.kill('SIGTERM')
        setTimeout(() => {
          if (!childProcess.killed) {
            childProcess.kill('SIGKILL')
          }
          finish()
        }, 1000)
      })
    })
  }

  private handleStdoutLine(line: string): void {
    const normalizedLine = line.trim()
    if (normalizedLine.length === 0) {
      return
    }

    try {
      const message = JSON.parse(normalizedLine) as Record<string, unknown>
      if (message.kind === 'runtimeEvent') {
        this.lastRuntimeEvent = message.event ?? null
      }
      this.listeners?.onMessage(message)
    } catch (error) {
      this.forwardError(error)
    }
  }

  private forwardError(error: unknown): void {
    if (this.shuttingDown) {
      return
    }
    this.listeners?.onError(error)
  }

  private disposeReaders(): void {
    this.stdoutReader?.close()
    this.stdoutReader = null
    this.stderrReader?.close()
    this.stderrReader = null
  }
}

function isXbxEngineStderrImportant(line: string): boolean {
  const normalized = line.toLowerCase()
  return (
    normalized.includes('panic') ||
    normalized.includes('error') ||
    normalized.includes('failed') ||
    normalized.includes('fatal')
  )
}

function loadXbxEngineNativeAddon(): XbxEngineNativeAddonModule | null {
  const attemptedPaths: string[] = []

  for (const candidatePath of resolveXbxEngineNativeCandidates()) {
    attemptedPaths.push(candidatePath)
    if (!fs.existsSync(candidatePath)) {
      continue
    }

    try {
      return loadAddonFromFile(candidatePath)
    } catch (error) {
      console.warn('[main][streaming] failed to load XbxEngine native addon', {
        candidatePath,
        error
      })
    }
  }

  if (attemptedPaths.length > 0) {
    console.warn('[main][streaming] XbxEngine native addon not found', {
      attemptedPaths
    })
  }
  return null
}

function createStdioXbxEngineBinding(): XbxEngineNativeBinding | null {
  const appBinaryPath = resolveXbxEngineAppBinaryPath()
  if (appBinaryPath === null) {
    return null
  }

  return new StdioXbxEngineBinding(appBinaryPath, ['--stdio'])
}

function resolveXbxEngineRuntimeConfig(): Record<string, unknown> {
  const fileConfig = loadRuntimeConfigFile()
  const mergedConfig = {
    runtimeName:
      typeof fileConfig.runtimeName === 'string' && fileConfig.runtimeName.trim().length > 0
        ? fileConfig.runtimeName
        : 'rust-owned',
    webrtc: {
      // 强控码率仅接受 main 环境变量，不从文件继承，避免历史字段残留误限速。
      forcedRembKbps: parseEnvNumber(XBXENGINE_FORCE_REMB_KBPS_ENV),
      adaptiveRembEnabled: parseEnvBoolean(XBXENGINE_ADAPTIVE_REMB_ENABLED_ENV) ?? true,
      videoPipeline: {
        ...(isRecord(fileConfig.webrtc) && isRecord(fileConfig.webrtc.videoPipeline)
          ? fileConfig.webrtc.videoPipeline
          : {}),
        nackWindowMs:
          parseEnvNumber(XBXENGINE_NACK_WINDOW_MS_ENV) ??
          parseConfigNumber(fileConfig, ['webrtc', 'videoPipeline', 'nackWindowMs']) ??
          400,
        nackRetryIntervalMs:
          parseEnvNumber(XBXENGINE_NACK_RETRY_INTERVAL_MS_ENV) ??
          parseConfigNumber(fileConfig, ['webrtc', 'videoPipeline', 'nackRetryIntervalMs']) ??
          60,
        nackMaxRetryCount:
          parseEnvNumber(XBXENGINE_NACK_MAX_RETRY_COUNT_ENV) ??
          parseConfigNumber(fileConfig, ['webrtc', 'videoPipeline', 'nackMaxRetryCount']) ??
          5
      },
      rttDiagnostics: {
        ...(isRecord(fileConfig.webrtc) && isRecord(fileConfig.webrtc.rttDiagnostics)
          ? fileConfig.webrtc.rttDiagnostics
          : {}),
        enabled:
          parseEnvBoolean(XBXENGINE_RTT_DIAGNOSTICS_ENABLED_ENV) ??
          parseConfigBoolean(fileConfig, ['webrtc', 'rttDiagnostics', 'enabled']) ??
          true,
        logIntervalMs:
          parseEnvNumber(XBXENGINE_RTT_DIAGNOSTICS_LOG_INTERVAL_MS_ENV) ??
          parseConfigNumber(fileConfig, ['webrtc', 'rttDiagnostics', 'logIntervalMs']) ??
          5000
      }
    }
  }
  return mergedConfig
}

function loadRuntimeConfigFile(): Record<string, unknown> {
  const configuredPath = process.env[XBXENGINE_RUNTIME_CONFIG_PATH_ENV]
  const configPath =
    configuredPath !== undefined && configuredPath.trim().length > 0
      ? path.resolve(configuredPath)
      : path.resolve(process.cwd(), 'resources/xbxengine.runtime.json')
  if (!fs.existsSync(configPath)) {
    return {}
  }
  try {
    const raw = fs.readFileSync(configPath, 'utf8')
    const parsed = JSON.parse(raw) as unknown
    if (isRecord(parsed)) {
      return parsed
    }
  } catch (error) {
    console.warn('[main][streaming] failed to load xbxengine runtime config file', {
      configPath,
      error
    })
  }
  return {}
}

function parseEnvNumber(key: string): number | null {
  const value = process.env[key]
  if (value === undefined || value.trim().length === 0) {
    return null
  }
  const parsed = Number(value)
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null
}

function parseEnvBoolean(key: string): boolean | null {
  const value = process.env[key]
  if (value === undefined || value.trim().length === 0) {
    return null
  }
  const normalized = value.trim().toLowerCase()
  if (['1', 'true', 'on', 'yes'].includes(normalized)) {
    return true
  }
  if (['0', 'false', 'off', 'no'].includes(normalized)) {
    return false
  }
  return null
}

function parseConfigNumber(source: Record<string, unknown>, pathChain: string[]): number | null {
  const value = readNestedValue(source, pathChain)
  return typeof value === 'number' && Number.isFinite(value) ? value : null
}

function parseConfigBoolean(source: Record<string, unknown>, pathChain: string[]): boolean | null {
  const value = readNestedValue(source, pathChain)
  return typeof value === 'boolean' ? value : null
}

function readNestedValue(source: Record<string, unknown>, pathChain: string[]): unknown {
  let current: unknown = source
  for (const key of pathChain) {
    if (!isRecord(current) || !(key in current)) {
      return null
    }
    current = current[key]
  }
  return current
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function resolveXbxEngineNativeCandidates(): string[] {
  const candidates = new Set<string>()
  const envPath = process.env.XBXENGINE_NAPI_PATH ?? process.env.STREAMSIDECAR_NAPI_PATH
  if (envPath) {
    candidates.add(path.resolve(envPath))
  }

  const workspaceRoot = process.cwd()
  const nativeFileNames = resolveNativeFileNames()
  const targetDirs = [
    path.resolve(workspaceRoot, 'target', 'debug'),
    path.resolve(workspaceRoot, 'target', 'release')
  ]

  targetDirs.forEach((targetDir) => {
    nativeFileNames.forEach((fileName) => {
      candidates.add(path.join(targetDir, fileName))
    })
  })

  return [...candidates]
}

function resolveXbxEngineAppBinaryPath(): string | null {
  const attemptedPaths: string[] = []

  for (const candidatePath of resolveXbxEngineAppCandidates()) {
    attemptedPaths.push(candidatePath)
    if (fs.existsSync(candidatePath)) {
      return candidatePath
    }
  }

  if (attemptedPaths.length > 0) {
    console.warn('[main][streaming] XbxEngine app binary not found', {
      attemptedPaths
    })
  }

  return null
}

function resolveXbxEngineAppCandidates(): string[] {
  const candidates = new Set<string>()
  const envPath =
    process.env[XBXENGINE_APP_PATH_ENV] ?? process.env[LEGACY_STREAMSIDECAR_APP_PATH_ENV]
  if (envPath) {
    candidates.add(path.resolve(envPath))
  }

  const workspaceRoot = process.cwd()
  const fileNames =
    process.platform === 'win32'
      ? ['xbxengine-app.exe', 'streamsidecar-app.exe']
      : ['xbxengine-app', 'streamsidecar-app']
  const targetDirs = [
    path.resolve(workspaceRoot, 'target', 'debug'),
    path.resolve(workspaceRoot, 'target', 'release')
  ]

  targetDirs.forEach((targetDir) => {
    fileNames.forEach((fileName) => {
      candidates.add(path.join(targetDir, fileName))
    })
  })

  return [...candidates]
}

function resolveNativeFileNames(): string[] {
  switch (process.platform) {
    case 'darwin':
      return ['libxbxengine_api.dylib', 'xbxengine_api.node']
    case 'win32':
      return ['xbxengine_api.dll', 'xbxengine_api.node']
    default:
      return ['libxbxengine_api.so', 'xbxengine_api.node']
  }
}

function loadAddonFromFile(filePath: string): XbxEngineNativeAddonModule {
  if (filePath.endsWith('.node')) {
    return requireNativeModule(filePath) as XbxEngineNativeAddonModule
  }

  const nativeModule = { exports: {} as XbxEngineNativeAddonModule }
  process.dlopen(nativeModule, filePath)
  return nativeModule.exports
}
