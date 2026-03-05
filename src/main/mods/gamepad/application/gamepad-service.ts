import {
  DEFAULT_GAMEPAD_SAMPLING_CONFIG_DTO,
  type GamepadDeviceDto,
  type GamepadRouteTargetDto,
  type GamepadRuntimeSnapshotDto,
  type GamepadSamplingConfigDto,
  type LogicalPadBindingDto,
  type LogicalPadSnapshotDto
} from '../../../../shared/gamepad/contract'

type RuntimeSnapshotListener = (snapshot: GamepadRuntimeSnapshotDto) => void
type DevicesChangedListener = (devices: GamepadDeviceDto[]) => void
type PadSnapshotListener = (snapshot: LogicalPadSnapshotDto) => void
type RouteChangedListener = (target: GamepadRouteTargetDto) => void

function cloneDto<T>(value: T): T {
  return structuredClone(value)
}

function createDefaultRuntimeSnapshot(): GamepadRuntimeSnapshotDto {
  return {
    devices: [],
    bindings: [],
    routeTarget: { kind: 'shell-ui' },
    sampling: cloneDto(DEFAULT_GAMEPAD_SAMPLING_CONFIG_DTO),
    pads: []
  }
}

/**
 * gamepad 域先由主进程维护一份内存态运行快照。
 * 后续 Rust bridge 接入后，仍通过这里向 RPC 与事件桥暴露统一语义。
 */
export class GamepadService {
  private runtimeSnapshot = createDefaultRuntimeSnapshot()
  private readonly runtimeSnapshotListeners = new Set<RuntimeSnapshotListener>()
  private readonly devicesChangedListeners = new Set<DevicesChangedListener>()
  private readonly padSnapshotListeners = new Set<PadSnapshotListener>()
  private readonly routeChangedListeners = new Set<RouteChangedListener>()

  getRuntimeSnapshot(): GamepadRuntimeSnapshotDto {
    return cloneDto(this.runtimeSnapshot)
  }

  setRouteTarget(target: GamepadRouteTargetDto): GamepadRuntimeSnapshotDto {
    this.runtimeSnapshot = {
      ...this.runtimeSnapshot,
      routeTarget: cloneDto(target)
    }
    this.emitRouteChanged(target)
    this.emitRuntimeSnapshot()
    return this.getRuntimeSnapshot()
  }

  updateSampling(sampling: GamepadSamplingConfigDto): GamepadRuntimeSnapshotDto {
    this.runtimeSnapshot = {
      ...this.runtimeSnapshot,
      sampling: cloneDto(sampling)
    }
    this.emitRuntimeSnapshot()
    return this.getRuntimeSnapshot()
  }

  rebindLogicalPad(binding: LogicalPadBindingDto): GamepadRuntimeSnapshotDto {
    const bindings = cloneDto(this.runtimeSnapshot.bindings)
    const index = bindings.findIndex((item) => item.padId === binding.padId)
    if (index >= 0) {
      bindings[index] = cloneDto(binding)
    } else {
      bindings.push(cloneDto(binding))
    }

    this.runtimeSnapshot = {
      ...this.runtimeSnapshot,
      bindings
    }
    this.emitRuntimeSnapshot()
    return this.getRuntimeSnapshot()
  }

  replaceDevices(devices: GamepadDeviceDto[]): void {
    this.runtimeSnapshot = {
      ...this.runtimeSnapshot,
      devices: cloneDto(devices)
    }
    this.emitDevicesChanged(devices)
    this.emitRuntimeSnapshot()
  }

  pushPadSnapshot(snapshot: LogicalPadSnapshotDto): void {
    const pads = cloneDto(this.runtimeSnapshot.pads)
    const index = pads.findIndex((item) => item.padId === snapshot.padId)
    if (index >= 0) {
      pads[index] = cloneDto(snapshot)
    } else {
      pads.push(cloneDto(snapshot))
    }

    this.runtimeSnapshot = {
      ...this.runtimeSnapshot,
      pads
    }
    this.emitPadSnapshot(snapshot)
    this.emitRuntimeSnapshot()
  }

  replaceRuntimeSnapshot(snapshot: GamepadRuntimeSnapshotDto): void {
    const previousRouteTarget = this.runtimeSnapshot.routeTarget
    this.runtimeSnapshot = cloneDto(snapshot)
    this.emitDevicesChanged(snapshot.devices)
    if (JSON.stringify(previousRouteTarget) !== JSON.stringify(snapshot.routeTarget)) {
      this.emitRouteChanged(snapshot.routeTarget)
    }
    snapshot.pads.forEach((padSnapshot) => {
      this.emitPadSnapshot(padSnapshot)
    })
    this.emitRuntimeSnapshot()
  }

  onRuntimeSnapshot(listener: RuntimeSnapshotListener): () => void {
    this.runtimeSnapshotListeners.add(listener)
    return () => {
      this.runtimeSnapshotListeners.delete(listener)
    }
  }

  onDevicesChanged(listener: DevicesChangedListener): () => void {
    this.devicesChangedListeners.add(listener)
    return () => {
      this.devicesChangedListeners.delete(listener)
    }
  }

  onPadSnapshot(listener: PadSnapshotListener): () => void {
    this.padSnapshotListeners.add(listener)
    return () => {
      this.padSnapshotListeners.delete(listener)
    }
  }

  onRouteChanged(listener: RouteChangedListener): () => void {
    this.routeChangedListeners.add(listener)
    return () => {
      this.routeChangedListeners.delete(listener)
    }
  }

  private emitRuntimeSnapshot(): void {
    const snapshot = this.getRuntimeSnapshot()
    this.runtimeSnapshotListeners.forEach((listener) => {
      listener(snapshot)
    })
  }

  private emitDevicesChanged(devices: GamepadDeviceDto[]): void {
    const payload = cloneDto(devices)
    this.devicesChangedListeners.forEach((listener) => {
      listener(payload)
    })
  }

  private emitPadSnapshot(snapshot: LogicalPadSnapshotDto): void {
    const payload = cloneDto(snapshot)
    this.padSnapshotListeners.forEach((listener) => {
      listener(payload)
    })
  }

  private emitRouteChanged(target: GamepadRouteTargetDto): void {
    const payload = cloneDto(target)
    this.routeChangedListeners.forEach((listener) => {
      listener(payload)
    })
  }
}
