import type { RuntimeFeatureCapabilities } from '../config/runtime-features'
import type { ConsoleLibrary, LibraryAchievement, LibraryGame, LibraryMedia, LibraryPage, MediaKind, PersonalAccount, Playtime, QueryResult } from './library'
import type {
  RuntimeTraceAckResult,
  RuntimeTraceExport,
  RuntimeTraceFlushResult,
  RuntimeTraceProducerPolicy,
  RuntimeTraceRecordEventParams,
  RuntimeTraceStatus,
} from './runtimeTrace'
import type {
  StreamingCancelStartupParams,
  StreamingCancelStartupResult,
  StreamingCloseSessionParams,
  StreamingCloseSessionResult,
  StreamingDecideRecoveryParams,
  StreamingDecideRecoveryResult,
  StreamingExchangeOfferParams,
  StreamingExchangeOfferResult,
  StreamingGetSessionProgressParams,
  StreamingListActiveSessionsParams,
  StreamingListActiveSessionsResult,
  StreamingPollIceParams,
  StreamingPollIceResult,
  StreamingSessionProgressSnapshot,
  StreamingStartSessionParams,
  StreamingStartSessionResult,
  StreamingSubmitIceParams,
  StreamingSubmitIceResult,
} from './streaming'
import type { RpcMethod } from './types'

type RpcConfigGroup = Record<string, unknown>

export interface XBoxRpcSchema {
  app: {
    getVersion: RpcMethod<void, string>
    ping: RpcMethod<{ message: string }, { message: string, at: string }>
    isFullscreen: RpcMethod<void, boolean>
    toggleFullscreen: RpcMethod<void, boolean>
    enterFullscreen: RpcMethod<void, boolean>
    exitFullscreen: RpcMethod<void, boolean>
    getStartupFlags: RpcMethod<void, { fullscreen: boolean, autoConnect: string }>
    resetAutoConnect: RpcMethod<void, { reset: boolean }>
    clearUserData: RpcMethod<void, { cleared: boolean }>
    clearData: RpcMethod<
      void,
      { cleared: boolean, legacyStateCleared: boolean, restarted: boolean }
    >
    saveBinaryFile: RpcMethod<
      {
        suggestedName: string
        dataBase64: string
      },
      { saved: boolean, canceled: boolean, path: string | null }
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
    checkAuthentication: RpcMethod<void, { provider: 'xal' | 'msal', startedSilentFlow: boolean }>
    login: RpcMethod<
      void,
      {
        provider: 'xal' | 'msal'
        mode: 'oauth-window' | 'device-code'
        oauth?: { url: string, state: string }
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
      { cleared: boolean, scope: 'ephemeral' | 'all' }
    >
    logout: RpcMethod<void, { loggedOut: boolean }>
  }
  config: {
    getRuntimeFeatures: RpcMethod<void, RuntimeFeatureCapabilities>
    get: RpcMethod<{ keys: string[] }, unknown>
    set: RpcMethod<{ patch: Record<string, unknown> }, unknown>
    getGroups: RpcMethod<
      void,
      {
        app: RpcConfigGroup
        streaming: RpcConfigGroup
        host: RpcConfigGroup
        xcloud: RpcConfigGroup
        input: RpcConfigGroup
      }
    >
  }
  data: {
    getLibraryGame: RpcMethod<{ targetXuid?: string, xboxTitleId: string }, QueryResult<LibraryGame | null>>
    getPersonalAccount: RpcMethod<{ targetXuid?: string }, QueryResult<PersonalAccount>>
    getGameHistory: RpcMethod<{ targetXuid?: string, continuation?: string }, QueryResult<LibraryPage<LibraryGame>>>
    getAchievements: RpcMethod<{ targetXuid?: string, xboxTitleId: string, continuation?: string, language: string }, QueryResult<LibraryPage<LibraryAchievement>>>
    getPlaytimes: RpcMethod<{ targetXuid?: string, xboxTitleIds: string[] }, QueryResult<Playtime[]>>
    getConsoleLibrary: RpcMethod<{ consoleId: string }, QueryResult<ConsoleLibrary>>
    getMedia: RpcMethod<{ kind: MediaKind, continuation?: string }, QueryResult<LibraryPage<LibraryMedia>>>
    resolveMedia: RpcMethod<{ kind: MediaKind, id: string, continuation?: string }, QueryResult<LibraryMedia>>
    saveMedia: RpcMethod<{ kind: MediaKind, id: string, continuation?: string }, { saved: boolean, canceled: boolean }>

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
      { xboxTitleId: string, config: Record<string, unknown> }
    >
    powerOnConsole: RpcMethod<{ consoleId: string }, { consoleId: string, accepted: boolean }>
    powerOffConsole: RpcMethod<{ consoleId: string }, { consoleId: string, accepted: boolean }>
    sendTextToConsole: RpcMethod<
      { consoleId: string, text: string },
      { consoleId: string, accepted: boolean }
    >
    getXcloudTitles: RpcMethod<
      void,
      {
        titles: Array<{
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
        scopeKey?: string
        cacheState: 'miss' | 'fresh' | 'stale'
        updatedAt?: number
        refreshing: boolean
      }
    >
    refreshXcloudTitles: RpcMethod<
      void,
      {
        titles: Array<{
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
        scopeKey?: string
        cacheState: 'miss' | 'fresh' | 'stale'
        updatedAt?: number
        refreshing: boolean
      }
    >
    primeXcloudTitles: RpcMethod<void, boolean>
  }
  streaming: {
    startSession: RpcMethod<StreamingStartSessionParams, StreamingStartSessionResult>
    cancelStartup: RpcMethod<StreamingCancelStartupParams, StreamingCancelStartupResult>
    getSessionProgress: RpcMethod<
      StreamingGetSessionProgressParams,
      StreamingSessionProgressSnapshot | null
    >
    closeSession: RpcMethod<StreamingCloseSessionParams, StreamingCloseSessionResult>
    exchangeOffer: RpcMethod<StreamingExchangeOfferParams, StreamingExchangeOfferResult>
    submitIce: RpcMethod<StreamingSubmitIceParams, StreamingSubmitIceResult>
    pollIce: RpcMethod<StreamingPollIceParams, StreamingPollIceResult>
    listActiveSessions: RpcMethod<
      StreamingListActiveSessionsParams,
      StreamingListActiveSessionsResult
    >
    decideRecovery: RpcMethod<StreamingDecideRecoveryParams, StreamingDecideRecoveryResult>
  }
  runtimeTrace: {
    recordEvent: RpcMethod<RuntimeTraceRecordEventParams, RuntimeTraceAckResult>
    getPolicy: RpcMethod<void, RuntimeTraceProducerPolicy>
    flush: RpcMethod<void, RuntimeTraceFlushResult>
    getStatus: RpcMethod<void, RuntimeTraceStatus>
    exportBundle: RpcMethod<void, RuntimeTraceExport>
  }
  system: {
    openExternal: RpcMethod<{ url: string }, void>
  }
  updater: {
    getChannel: RpcMethod<void, { channel: 'stable' | 'beta' }>
    setChannel: RpcMethod<{ channel: 'stable' | 'beta' }, { channel: 'stable' | 'beta' }>
    check: RpcMethod<
      void,
      {
        currentVersion: string
        updateAvailable: boolean
        version: string | null
        notes: string | null
        date: string | null
      }
    >
    downloadAndInstall: RpcMethod<void, { accepted: boolean }>
    relaunch: RpcMethod<void, { accepted: boolean }>
  }
}
