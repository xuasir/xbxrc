export interface HostSummary {
  id?: string
  deviceId?: string
  serverId?: string
  name?: string
  deviceName?: string
  locale?: string
  region?: string
  powerState?: string
  consoleType?: string
  remoteManagementEnabled?: boolean
  consoleStreamingEnabled?: boolean
  digitalAssistantRemoteControlEnabled?: boolean
  wirelessWarning?: boolean
  outOfHomeWarning?: boolean
  storageDevices?: Array<{
    storageDeviceId?: string
    storageDeviceName?: string
    id?: string
    name?: string
    freeSpaceBytes?: number
    freeBytes?: number
    totalSpaceBytes?: number
    totalBytes?: number
  }>
}

export interface HostsSnapshot {
  scopeKey: string
  hosts: HostSummary[]
  updatedAt: number
  freshUntil: number
}
