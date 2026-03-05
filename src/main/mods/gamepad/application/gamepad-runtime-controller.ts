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
import { GamepadService } from './gamepad-service'
import {
  createDefaultGamepadNativeBinding,
  type GamepadNativeBinding
} from './gamepad-native-binding'

export class GamepadRuntimeController {
  private readonly service = new GamepadService()
  private readonly nativeBinding: GamepadNativeBinding | null
  private startPromise?: Promise<void>

  constructor(nativeBinding: GamepadNativeBinding | null = createDefaultGamepadNativeBinding()) {
    this.nativeBinding = nativeBinding
  }

  getService(): GamepadService {
    this.ensureStarted()
    return this.service
  }

  async getRuntimeSnapshot(): Promise<GamepadRuntimeSnapshotDto> {
    await this.ensureStarted()
    if (!this.nativeBinding) {
      return this.service.getRuntimeSnapshot()
    }

    const snapshot = await this.nativeBinding.getRuntimeSnapshot()
    this.service.replaceRuntimeSnapshot(snapshot)
    return this.service.getRuntimeSnapshot()
  }

  async setRouteTarget(target: GamepadRouteTargetDto): Promise<GamepadRuntimeSnapshotDto> {
    await this.ensureStarted()
    if (!this.nativeBinding) {
      return this.service.setRouteTarget(target)
    }

    const snapshot = await this.nativeBinding.setRouteTarget(target)
    this.service.replaceRuntimeSnapshot(snapshot)
    return this.service.getRuntimeSnapshot()
  }

  async updateSampling(sampling: GamepadSamplingConfigDto): Promise<GamepadRuntimeSnapshotDto> {
    await this.ensureStarted()
    if (!this.nativeBinding) {
      return this.service.updateSampling(sampling)
    }

    const snapshot = await this.nativeBinding.updateSampling(sampling)
    this.service.replaceRuntimeSnapshot(snapshot)
    return this.service.getRuntimeSnapshot()
  }

  async rebindLogicalPad(binding: LogicalPadBindingDto): Promise<GamepadRuntimeSnapshotDto> {
    await this.ensureStarted()
    if (!this.nativeBinding) {
      return this.service.rebindLogicalPad(binding)
    }

    const snapshot = await this.nativeBinding.rebindLogicalPad(binding)
    this.service.replaceRuntimeSnapshot(snapshot)
    return this.service.getRuntimeSnapshot()
  }

  async setSamplingStrategy(
    strategy: GamepadSamplingStrategyDto
  ): Promise<GamepadRuntimeSnapshotDto> {
    await this.ensureStarted()
    if (!this.nativeBinding) {
      return this.service.getRuntimeSnapshot()
    }

    const snapshot = await this.nativeBinding.setSamplingStrategy(strategy)
    this.service.replaceRuntimeSnapshot(snapshot)
    return this.service.getRuntimeSnapshot()
  }

  async setPrimarySamplingDevice(deviceId: string | null): Promise<GamepadRuntimeSnapshotDto> {
    await this.ensureStarted()
    if (!this.nativeBinding) {
      return this.service.getRuntimeSnapshot()
    }

    const snapshot = await this.nativeBinding.setPrimarySamplingDevice(deviceId)
    this.service.replaceRuntimeSnapshot(snapshot)
    return this.service.getRuntimeSnapshot()
  }

  async pauseSamplingDevice(deviceId: string): Promise<GamepadRuntimeSnapshotDto> {
    await this.ensureStarted()
    if (!this.nativeBinding) {
      return this.service.getRuntimeSnapshot()
    }

    const snapshot = await this.nativeBinding.pauseSamplingDevice(deviceId)
    this.service.replaceRuntimeSnapshot(snapshot)
    return this.service.getRuntimeSnapshot()
  }

  async resumeSamplingDevice(deviceId: string): Promise<GamepadRuntimeSnapshotDto> {
    await this.ensureStarted()
    if (!this.nativeBinding) {
      return this.service.getRuntimeSnapshot()
    }

    const snapshot = await this.nativeBinding.resumeSamplingDevice(deviceId)
    this.service.replaceRuntimeSnapshot(snapshot)
    return this.service.getRuntimeSnapshot()
  }

  async playRumble(request: GamepadRumbleRequestDto): Promise<GamepadRumbleResultDto> {
    await this.ensureStarted()
    if (!this.nativeBinding) {
      return {
        accepted: false,
        reason: 'not-implemented',
        resolvedDeviceIds: []
      }
    }

    return await this.nativeBinding.playRumble(request)
  }

  async stopRumble(target: GamepadRumbleTargetDto): Promise<GamepadRumbleResultDto> {
    await this.ensureStarted()
    if (!this.nativeBinding) {
      return {
        accepted: false,
        reason: 'not-implemented',
        resolvedDeviceIds: []
      }
    }

    return await this.nativeBinding.stopRumble(target)
  }

  async shutdown(): Promise<void> {
    if (!this.nativeBinding) {
      return
    }
    await this.nativeBinding.shutdown()
  }

  private ensureStarted(): Promise<void> {
    if (this.startPromise) {
      return this.startPromise
    }

    this.startPromise = this.nativeBinding
      ? this.nativeBinding
          .start({
            onRuntimeSnapshot: (snapshot) => {
              this.service.replaceRuntimeSnapshot(snapshot)
            },
            onError: (error) => {
              console.warn('[main][gamepad] native binding error', error)
            }
          })
          .catch((error) => {
            console.warn('[main][gamepad] native binding bootstrap failed', error)
          })
      : Promise.resolve()

    return this.startPromise
  }
}
