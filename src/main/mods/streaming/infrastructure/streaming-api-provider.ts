import type { StreamingConfigPort } from '../domain/config-port'
import type {
  StreamingTargetType,
  StreamingTokenEnvelope
} from '../domain/types'
import { StreamingHttpClient } from './streaming-http-client'
import { StreamingIceNormalizer } from './streaming-ice-normalizer'
import { StreamingSessionApi } from './streaming-session-api'
import { StreamingSignalingApi } from './streaming-signaling-api'

interface StreamingApiProviderDeps {
  configPort: StreamingConfigPort
}

interface StreamingInfrastructureContext {
  sessionApi: StreamingSessionApi
  signalingApi: StreamingSignalingApi
  cacheKey: string
}

function resolveDefaultHost(token: StreamingTokenEnvelope, type: StreamingTargetType): string {
  const defaultRegion =
    token.data?.offeringSettings?.regions?.find((region) => region.isDefault) ??
    token.data?.offeringSettings?.regions?.[0]

  if (defaultRegion === undefined || typeof defaultRegion.baseUri !== 'string') {
    throw new Error(`Streaming region is missing for ${type}.`)
  }

  return defaultRegion.baseUri.replace(/^https?:\/\//, '')
}

function resolveGsToken(token: StreamingTokenEnvelope, type: StreamingTargetType): string {
  const gsToken = token.data?.gsToken
  if (typeof gsToken !== 'string' || gsToken.length === 0) {
    throw new Error(`Streaming gsToken is missing for ${type}.`)
  }
  return gsToken
}

// 串流基础设施 provider：按目标类型与 token/config 快照复用底层客户端。
export class StreamingApiProvider {
  private readonly configPort: StreamingConfigPort
  private readonly contexts = new Map<StreamingTargetType, StreamingInfrastructureContext>()

  constructor(deps: StreamingApiProviderDeps) {
    this.configPort = deps.configPort
  }

  private resolveOsName(): string {
    const { resolution } = this.configPort.getStreamingConfig()
    if (resolution === 1081) {
      return 'tizen'
    }
    if (resolution === 1080) {
      return 'windows'
    }
    return 'android'
  }

  private createDeviceInfo(osName: string): string {
    return JSON.stringify({
      appInfo: {
        env: {
          clientAppId: 'www.xbox.com',
          clientAppType: 'browser',
          clientAppVersion: '26.1.97',
          clientSdkVersion: '10.3.7',
          httpEnvironment: 'prod',
          sdkInstallId: ''
        }
      },
      dev: {
        hw: {
          make: 'Microsoft',
          model: 'unknown',
          sdktype: 'web'
        },
        os: {
          name: osName,
          ver: '22631.2715',
          platform: 'desktop'
        },
        displayInfo: {
          dimensions: {
            widthInPixels: 1920,
            heightInPixels: 1080
          },
          pixelDensity: {
            dpiX: 1,
            dpiY: 1
          }
        },
        browser: {
          browserName: 'chrome',
          browserVersion: '130.0'
        }
      }
    })
  }

  private createCacheKey(token: StreamingTokenEnvelope, type: StreamingTargetType): string {
    const config = this.configPort.getStreamingConfig()
    return JSON.stringify({
      type,
      host: resolveDefaultHost(token, type),
      gsToken: resolveGsToken(token, type),
      resolution: config.resolution,
      preferredGameLanguage: config.preferred_game_language,
      ipv6: config.ipv6
    })
  }

  private createContext(token: StreamingTokenEnvelope, type: StreamingTargetType): StreamingInfrastructureContext {
    const host = resolveDefaultHost(token, type)
    const gsToken = resolveGsToken(token, type)
    const config = this.configPort.getStreamingConfig()
    const httpClient = new StreamingHttpClient({
      host,
      bearerToken: gsToken
    })

    return {
      cacheKey: this.createCacheKey(token, type),
      sessionApi: new StreamingSessionApi({
        type,
        httpClient,
        createDeviceInfo: (osName) => this.createDeviceInfo(osName),
        resolveOsName: () => this.resolveOsName(),
        preferredGameLanguage: config.preferred_game_language
      }),
      signalingApi: new StreamingSignalingApi({
        sessionBasePath: `/v5/sessions/${type}`,
        httpClient,
        iceNormalizer: new StreamingIceNormalizer({
          ipv6: config.ipv6
        })
      })
    }
  }

  private getContext(token: StreamingTokenEnvelope, type: StreamingTargetType): StreamingInfrastructureContext {
    const nextCacheKey = this.createCacheKey(token, type)
    const cached = this.contexts.get(type)
    if (cached !== undefined && cached.cacheKey === nextCacheKey) {
      return cached
    }

    const nextContext = this.createContext(token, type)
    this.contexts.set(type, nextContext)
    return nextContext
  }

  getSessionApi(token: StreamingTokenEnvelope, type: StreamingTargetType): StreamingSessionApi {
    return this.getContext(token, type).sessionApi
  }

  getSignalingApi(token: StreamingTokenEnvelope, type: StreamingTargetType): StreamingSignalingApi {
    return this.getContext(token, type).signalingApi
  }
}
