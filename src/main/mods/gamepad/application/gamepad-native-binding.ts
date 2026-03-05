import fs from 'node:fs'
import { createRequire } from 'node:module'
import path from 'node:path'
import process from 'node:process'
import type {
  GamepadRouteTargetDto,
  GamepadRumbleRequestDto,
  GamepadRumbleResultDto,
  GamepadRumbleTargetDto,
  GamepadRuntimeSnapshotDto,
  GamepadSamplingStrategyDto,
  GamepadSamplingConfigDto,
  LogicalPadBindingDto
} from '../../../../shared/gamepad/contract'

export interface GamepadNativeBindingListeners {
  onRuntimeSnapshot(snapshot: GamepadRuntimeSnapshotDto): void
  onError(error: unknown): void
}

/**
 * 主进程只认这一层接口。
 * Rust 侧当前由 `xbxengine-api` 暴露统一 napi 门面，
 * `ohmygamepad-bridge-napi` 仅保留为禁用的兼容路径。
 */
export interface GamepadNativeBinding {
  start(listeners: GamepadNativeBindingListeners): Promise<void>
  getRuntimeSnapshot(): Promise<GamepadRuntimeSnapshotDto>
  setRouteTarget(target: GamepadRouteTargetDto): Promise<GamepadRuntimeSnapshotDto>
  updateSampling(sampling: GamepadSamplingConfigDto): Promise<GamepadRuntimeSnapshotDto>
  rebindLogicalPad(binding: LogicalPadBindingDto): Promise<GamepadRuntimeSnapshotDto>
  setSamplingStrategy(strategy: GamepadSamplingStrategyDto): Promise<GamepadRuntimeSnapshotDto>
  setPrimarySamplingDevice(deviceId: string | null): Promise<GamepadRuntimeSnapshotDto>
  pauseSamplingDevice(deviceId: string): Promise<GamepadRuntimeSnapshotDto>
  resumeSamplingDevice(deviceId: string): Promise<GamepadRuntimeSnapshotDto>
  playRumble(request: GamepadRumbleRequestDto): Promise<GamepadRumbleResultDto>
  stopRumble(target: GamepadRumbleTargetDto): Promise<GamepadRumbleResultDto>
  pressControllerButton(button: string, durationMs: number): Promise<GamepadRuntimeSnapshotDto>
  shutdown(): Promise<void>
}

interface XbxEngineGamepadNativeAddonBinding {
  getRuntimeSnapshotJson(): string
  subscribeRuntimeSnapshot?(
    onSnapshotJson: (snapshotJson: string) => void,
    onErrorMessage?: (errorMessage: string) => void
  ): void
  unsubscribeRuntimeSnapshot?(): void
  setRouteTargetJson(targetJson: string): string
  updateSamplingJson(samplingJson: string): string
  rebindLogicalPadJson(bindingJson: string): string
  setSamplingStrategyJson(strategyJson: string): string
  setPrimarySamplingDeviceJson(deviceIdJson: string): string
  pauseSamplingDeviceJson(deviceIdJson: string): string
  resumeSamplingDeviceJson(deviceIdJson: string): string
  playRumbleJson(requestJson: string): string
  stopRumbleJson(targetJson: string): string
  pressControllerButtonJson(requestJson: string): string
  shutdown(): void
}

interface XbxEngineGamepadNativeAddonModule {
  XbxEngineGamepadNativeBinding: new () => XbxEngineGamepadNativeAddonBinding
}

const NATIVE_FALLBACK_POLL_INTERVAL_MS = 1000 / 60
const requireNativeModule = createRequire(import.meta.url)

export function createDefaultGamepadNativeBinding(): GamepadNativeBinding | null {
  const nativeModule = loadGamepadNativeAddon()
  if (!nativeModule) {
    return null
  }

  return new NapiGamepadNativeBinding(nativeModule)
}

class NapiGamepadNativeBinding implements GamepadNativeBinding {
  private readonly addonBinding: XbxEngineGamepadNativeAddonBinding
  private pollTimer: NodeJS.Timeout | null = null
  private runtimeSnapshotSubscribed = false
  private lastSnapshotJson: string | null = null

  constructor(nativeModule: XbxEngineGamepadNativeAddonModule) {
    this.addonBinding = new nativeModule.XbxEngineGamepadNativeBinding()
  }

  async start(listeners: GamepadNativeBindingListeners): Promise<void> {
    if (this.pollTimer || this.runtimeSnapshotSubscribed) {
      return
    }

    if (this.addonBinding.subscribeRuntimeSnapshot) {
      try {
        this.addonBinding.subscribeRuntimeSnapshot(
          (snapshotJson) => {
            try {
              this.emitSnapshotJson(snapshotJson, listeners)
            } catch (error) {
              listeners.onError(error)
            }
          },
          (errorMessage) => {
            listeners.onError(new Error(errorMessage))
          }
        )
        this.runtimeSnapshotSubscribed = true
        return
      } catch (error) {
        listeners.onError(error)
        return
      }
    }

    const emitSnapshot = (): void => {
      try {
        this.emitSnapshotJson(this.addonBinding.getRuntimeSnapshotJson(), listeners)
      } catch (error) {
        listeners.onError(error)
      }
    }

    emitSnapshot()
    this.pollTimer = setInterval(emitSnapshot, NATIVE_FALLBACK_POLL_INTERVAL_MS)
  }

  async getRuntimeSnapshot(): Promise<GamepadRuntimeSnapshotDto> {
    return parseSnapshotJson(this.addonBinding.getRuntimeSnapshotJson())
  }

  async setRouteTarget(target: GamepadRouteTargetDto): Promise<GamepadRuntimeSnapshotDto> {
    const snapshotJson = this.addonBinding.setRouteTargetJson(JSON.stringify(target))
    this.lastSnapshotJson = snapshotJson
    return parseSnapshotJson(snapshotJson)
  }

  async updateSampling(sampling: GamepadSamplingConfigDto): Promise<GamepadRuntimeSnapshotDto> {
    const snapshotJson = this.addonBinding.updateSamplingJson(JSON.stringify(sampling))
    this.lastSnapshotJson = snapshotJson
    return parseSnapshotJson(snapshotJson)
  }

  async rebindLogicalPad(binding: LogicalPadBindingDto): Promise<GamepadRuntimeSnapshotDto> {
    const snapshotJson = this.addonBinding.rebindLogicalPadJson(JSON.stringify(binding))
    this.lastSnapshotJson = snapshotJson
    return parseSnapshotJson(snapshotJson)
  }

  async setSamplingStrategy(
    strategy: GamepadSamplingStrategyDto
  ): Promise<GamepadRuntimeSnapshotDto> {
    const snapshotJson = this.addonBinding.setSamplingStrategyJson(JSON.stringify(strategy))
    this.lastSnapshotJson = snapshotJson
    return parseSnapshotJson(snapshotJson)
  }

  async setPrimarySamplingDevice(deviceId: string | null): Promise<GamepadRuntimeSnapshotDto> {
    const snapshotJson = this.addonBinding.setPrimarySamplingDeviceJson(JSON.stringify(deviceId))
    this.lastSnapshotJson = snapshotJson
    return parseSnapshotJson(snapshotJson)
  }

  async pauseSamplingDevice(deviceId: string): Promise<GamepadRuntimeSnapshotDto> {
    const snapshotJson = this.addonBinding.pauseSamplingDeviceJson(JSON.stringify(deviceId))
    this.lastSnapshotJson = snapshotJson
    return parseSnapshotJson(snapshotJson)
  }

  async resumeSamplingDevice(deviceId: string): Promise<GamepadRuntimeSnapshotDto> {
    const snapshotJson = this.addonBinding.resumeSamplingDeviceJson(JSON.stringify(deviceId))
    this.lastSnapshotJson = snapshotJson
    return parseSnapshotJson(snapshotJson)
  }

  async playRumble(request: GamepadRumbleRequestDto): Promise<GamepadRumbleResultDto> {
    return parseRumbleResultJson(this.addonBinding.playRumbleJson(JSON.stringify(request)))
  }

  async stopRumble(target: GamepadRumbleTargetDto): Promise<GamepadRumbleResultDto> {
    return parseRumbleResultJson(this.addonBinding.stopRumbleJson(JSON.stringify(target)))
  }

  async pressControllerButton(button: string, durationMs: number): Promise<GamepadRuntimeSnapshotDto> {
    const snapshotJson = this.addonBinding.pressControllerButtonJson(
      JSON.stringify({
        button,
        duration_ms: durationMs
      })
    )
    this.lastSnapshotJson = snapshotJson
    return parseSnapshotJson(snapshotJson)
  }

  async shutdown(): Promise<void> {
    if (this.runtimeSnapshotSubscribed) {
      this.addonBinding.unsubscribeRuntimeSnapshot?.()
      this.runtimeSnapshotSubscribed = false
    }
    if (this.pollTimer) {
      clearInterval(this.pollTimer)
      this.pollTimer = null
    }
    this.addonBinding.shutdown()
  }

  private emitSnapshotJson(
    snapshotJson: string,
    listeners: GamepadNativeBindingListeners
  ): void {
    if (snapshotJson === this.lastSnapshotJson) {
      return
    }
    this.lastSnapshotJson = snapshotJson
    listeners.onRuntimeSnapshot(parseSnapshotJson(snapshotJson))
  }
}

function parseSnapshotJson(snapshotJson: string): GamepadRuntimeSnapshotDto {
  return JSON.parse(snapshotJson) as GamepadRuntimeSnapshotDto
}

function parseRumbleResultJson(resultJson: string): GamepadRumbleResultDto {
  return JSON.parse(resultJson) as GamepadRumbleResultDto
}

function loadGamepadNativeAddon(): XbxEngineGamepadNativeAddonModule | null {
  const attemptedPaths: string[] = []

  for (const candidatePath of resolveGamepadNativeCandidates()) {
    attemptedPaths.push(candidatePath)
    if (!fs.existsSync(candidatePath)) {
      continue
    }

    try {
      return loadAddonFromFile(candidatePath)
    } catch (error) {
      console.warn('[main][gamepad] failed to load XbxEngine gamepad native addon', {
        candidatePath,
        error
      })
    }
  }

  if (attemptedPaths.length > 0) {
    console.warn('[main][gamepad] XbxEngine gamepad native addon not found', {
      attemptedPaths,
      suggestedBuildCommand: 'pnpm run cargo:build:xbxengine-api'
    })
  }
  return null
}

function resolveGamepadNativeCandidates(): string[] {
  const candidates = new Set<string>()
  const xbxEngineEnvPath = process.env.XBXENGINE_NAPI_PATH ?? process.env.STREAMSIDECAR_NAPI_PATH
  if (xbxEngineEnvPath) {
    candidates.add(path.resolve(xbxEngineEnvPath))
  }
  const legacyEnvPath = process.env.OHMYGAMEPAD_NAPI_PATH
  if (legacyEnvPath) {
    candidates.add(path.resolve(legacyEnvPath))
  }

  const workspaceRoot = process.cwd()
  const nativeFileNames = resolveGamepadNativeFileNames()
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

function resolveGamepadNativeFileNames(): string[] {
  switch (process.platform) {
    case 'darwin':
      return ['libxbxengine_api.dylib', 'xbxengine_api.node']
    case 'win32':
      return ['xbxengine_api.dll', 'xbxengine_api.node']
    default:
      return ['libxbxengine_api.so', 'xbxengine_api.node']
  }
}

function loadAddonFromFile(filePath: string): XbxEngineGamepadNativeAddonModule {
  if (filePath.endsWith('.node')) {
    return requireNativeModule(filePath) as XbxEngineGamepadNativeAddonModule
  }

  // 直接走 process.dlopen，允许开发期直接加载 cargo 产出的动态库，无需额外复制成 `.node`。
  const nativeModule = { exports: {} } as NodeModule & { exports: unknown }
  process.dlopen(nativeModule, filePath)
  return nativeModule.exports as XbxEngineGamepadNativeAddonModule
}
