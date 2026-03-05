import type { RpcMethod } from './types'
import type {
  GamepadRouteTargetDto,
  GamepadRumbleRequestDto,
  GamepadRumbleResultDto,
  GamepadRumbleTargetDto,
  GamepadRuntimeSnapshotDto,
  GamepadSamplingStrategyDto,
  GamepadSamplingConfigDto,
  LogicalPadBindingDto
} from '../gamepad/contract'
import type {
  StreamingCloseSessionParams,
  StreamingCreateSessionParams,
  StreamingExchangeIceParams,
  StreamingExchangeIceResult,
  StreamingExchangeOfferParams,
  StreamingExchangeOfferResult,
  StreamingGetSessionParams,
  StreamingKeepAliveParams,
  StreamingKeepAliveResult,
  StreamingListActiveSessionsParams,
  StreamingListActiveSessionsResult,
  StreamingSessionSnapshot,
  StreamingTargetType,
  StreamingTurnServerConfig
} from './streaming'
import type {
  StreamHostCloseSessionParams,
  StreamHostExchangeIceParams,
  StreamHostExchangeIceResult,
  StreamHostExchangeOfferParams,
  StreamHostExchangeOfferResult,
  StreamHostKeepAliveParams
} from './stream-host'
import type {
  XbxEngineAckResult,
  XbxEngineApplyDisplayStateParams,
  XbxEngineAttachViewportParams,
  XbxEngineKeyboardPointerEnabledParams,
  XbxEnginePressControllerButtonParams,
  XbxEnginePushInputParams,
  XbxEngineRequestReconnectParams,
  XbxEngineRuntimeEventDto,
  XbxEngineSetAudioVolumeParams,
  XbxEngineStartRuntimeParams,
  XbxEngineStatsDto
} from './xbxengine'

type RpcConfigGroup = Record<string, unknown>

export interface XBoxRpcSchema {
  app: {
    getVersion: RpcMethod<void, string>
    ping: RpcMethod<{ message: string }, { message: string; at: string }>
    isFullscreen: RpcMethod<void, boolean>
    toggleFullscreen: RpcMethod<void, boolean>
    enterFullscreen: RpcMethod<void, boolean>
    exitFullscreen: RpcMethod<void, boolean>
    getStartupFlags: RpcMethod<void, { fullscreen: boolean; autoConnect: string }>
    resetAutoConnect: RpcMethod<void, { reset: boolean }>
    clearUserData: RpcMethod<void, { cleared: boolean }>
    clearData: RpcMethod<
      void,
      { cleared: boolean; legacyStateCleared: boolean; restarted: boolean }
    >
    quit: RpcMethod<void, { accepted: boolean }>
    restart: RpcMethod<void, { accepted: boolean }>
  }
  auth: {
    getState: RpcMethod<
      void,
      {
        provider: 'xal' | 'msal'
        isAuthenticating: boolean
        isAuthenticated: boolean
        appLevel: number
      }
    >
    checkAuthentication: RpcMethod<void, { provider: 'xal' | 'msal'; startedSilentFlow: boolean }>
    login: RpcMethod<
      void,
      {
        provider: 'xal' | 'msal'
        mode: 'oauth-window' | 'device-code'
        oauth?: { url: string; state: string }
        deviceCode?: {
          userCode: string
          deviceCode: string
          verificationUri: string
          message: string
          expiresIn: number
          interval: number
        }
      }
    >
    clearAuthCache: RpcMethod<
      { scope: 'ephemeral' | 'all' },
      { cleared: boolean; scope: 'ephemeral' | 'all' }
    >
    logout: RpcMethod<void, { loggedOut: boolean }>
  }
  config: {
    get: RpcMethod<{ keys: string[] }, unknown>
    set: RpcMethod<{ patch: Record<string, unknown> }, unknown>
    getGroups: RpcMethod<
      void,
      {
        app: RpcConfigGroup
        auth: RpcConfigGroup
        streaming: RpcConfigGroup
        input: RpcConfigGroup
        xhome: RpcConfigGroup
      }
    >
  }
  gamepad: {
    getRuntimeSnapshot: RpcMethod<void, GamepadRuntimeSnapshotDto>
    setRouteTarget: RpcMethod<{ target: GamepadRouteTargetDto }, GamepadRuntimeSnapshotDto>
    updateSampling: RpcMethod<{ sampling: GamepadSamplingConfigDto }, GamepadRuntimeSnapshotDto>
    rebindLogicalPad: RpcMethod<{ binding: LogicalPadBindingDto }, GamepadRuntimeSnapshotDto>
    setSamplingStrategy: RpcMethod<
      { strategy: GamepadSamplingStrategyDto },
      GamepadRuntimeSnapshotDto
    >
    setPrimarySamplingDevice: RpcMethod<{ deviceId: string | null }, GamepadRuntimeSnapshotDto>
    pauseSamplingDevice: RpcMethod<{ deviceId: string }, GamepadRuntimeSnapshotDto>
    resumeSamplingDevice: RpcMethod<{ deviceId: string }, GamepadRuntimeSnapshotDto>
    playRumble: RpcMethod<{ request: GamepadRumbleRequestDto }, GamepadRumbleResultDto>
    stopRumble: RpcMethod<{ target: GamepadRumbleTargetDto }, GamepadRumbleResultDto>
  }
  data: {
    getUserProfile: RpcMethod<
      void,
      {
        signedIn: boolean
        gameDisplayName: string
        gameDisplayPicRaw: string
        gamertag: string
        gamerscore: string
        settings: Record<string, string>
        appLevel: number
      }
    >
    getHosts: RpcMethod<
      void,
      Array<{
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
      }>
    >
    getRemoteConsoles: RpcMethod<
      void,
      Array<{
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
      }>
    >
    getStreamingTitleInputConfig: RpcMethod<
      { xboxTitleId: string },
      { xboxTitleId: string; config: Record<string, unknown> }
    >
    powerOnConsole: RpcMethod<{ consoleId: string }, { consoleId: string; accepted: boolean }>
    powerOffConsole: RpcMethod<{ consoleId: string }, { consoleId: string; accepted: boolean }>
    sendTextToConsole: RpcMethod<
      { consoleId: string; text: string },
      { consoleId: string; accepted: boolean }
    >
    getXcloudTitles: RpcMethod<
      void,
      Array<{
        id: string
        name: string
        productId: string
        titleId: string
        xboxTitleId?: number
        publisherName: string
        description: string
        tileImageUrl: string
        posterImageUrl: string
        categories: string[]
        supportedInputTypes: string[]
        hasEntitlement: boolean
        isRecentlyPlayed: boolean
        isNew: boolean
      }>
    >
  }
  streaming: {
    getFallbackTurnServer: RpcMethod<
      { targetType: StreamingTargetType },
      StreamingTurnServerConfig | null
    >
    createSession: RpcMethod<StreamingCreateSessionParams, StreamingSessionSnapshot>
    getSession: RpcMethod<StreamingGetSessionParams, StreamingSessionSnapshot | null>
    closeSession: RpcMethod<StreamingCloseSessionParams, { closed: boolean }>
    exchangeOffer: RpcMethod<StreamingExchangeOfferParams, StreamingExchangeOfferResult>
    exchangeIce: RpcMethod<StreamingExchangeIceParams, StreamingExchangeIceResult>
    sendKeepAlive: RpcMethod<StreamingKeepAliveParams, StreamingKeepAliveResult>
    listActiveSessions: RpcMethod<
      StreamingListActiveSessionsParams,
      StreamingListActiveSessionsResult
    >
  }
  streamHost: {
    exchangeOffer: RpcMethod<StreamHostExchangeOfferParams, StreamHostExchangeOfferResult>
    exchangeIce: RpcMethod<StreamHostExchangeIceParams, StreamHostExchangeIceResult>
    keepAliveRemoteSession: RpcMethod<StreamHostKeepAliveParams, { accepted: boolean }>
    closeRemoteSession: RpcMethod<StreamHostCloseSessionParams, { closed: boolean }>
  }
  xbxEngine: {
    startRuntime: RpcMethod<XbxEngineStartRuntimeParams, XbxEngineAckResult>
    requestReconnect: RpcMethod<XbxEngineRequestReconnectParams, XbxEngineAckResult>
    stopRuntime: RpcMethod<void, XbxEngineAckResult>
    attachViewport: RpcMethod<XbxEngineAttachViewportParams, XbxEngineAckResult>
    detachViewport: RpcMethod<void, XbxEngineAckResult>
    applyDisplayState: RpcMethod<XbxEngineApplyDisplayStateParams, XbxEngineAckResult>
    pressControllerButton: RpcMethod<XbxEnginePressControllerButtonParams, XbxEngineAckResult>
    setKeyboardPointerEnabled: RpcMethod<
      XbxEngineKeyboardPointerEnabledParams,
      XbxEngineAckResult
    >
    pushKeyboardPointerInput: RpcMethod<XbxEnginePushInputParams, XbxEngineAckResult>
    setAudioVolume: RpcMethod<XbxEngineSetAudioVolumeParams, XbxEngineAckResult>
    startMicrophone: RpcMethod<void, XbxEngineAckResult>
    stopMicrophone: RpcMethod<void, XbxEngineAckResult>
    snapshotStats: RpcMethod<void, XbxEngineStatsDto>
    getLastRuntimeEvent: RpcMethod<void, XbxEngineRuntimeEventDto | null>
  }
  system: {
    openExternal: RpcMethod<{ url: string }, void>
  }
}
