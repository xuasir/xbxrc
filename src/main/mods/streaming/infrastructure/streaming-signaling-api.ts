import type { StreamingAnswerPayload, StreamingIceCandidate } from '../domain/types'
import { StreamingHttpClient } from './streaming-http-client'
import { StreamingIceNormalizer } from './streaming-ice-normalizer'

export interface StreamingExchangeResponse {
  exchangeResponse?: string
}

export interface StreamingSignalingApiDeps {
  sessionBasePath: string
  httpClient: StreamingHttpClient
  iceNormalizer: StreamingIceNormalizer
}

export class StreamingSignalingApi {
  private readonly sessionBasePath: string
  private readonly httpClient: StreamingHttpClient
  private readonly iceNormalizer: StreamingIceNormalizer

  constructor(deps: StreamingSignalingApiDeps) {
    this.sessionBasePath = deps.sessionBasePath
    this.httpClient = deps.httpClient
    this.iceNormalizer = deps.iceNormalizer
  }

  async sendSdp(sessionId: string, sdp: string): Promise<void> {
    const path = `${this.sessionBasePath}/${sessionId}/sdp`
    await this.httpClient.requestJson('POST', path, {
      messageType: 'offer',
      sdp,
      configuration: {
        chatConfiguration: {
          bytesPerSample: 2,
          expectedClipDurationMs: 20,
          format: {
            codec: 'opus',
            container: 'webm'
          },
          numChannels: 1,
          sampleFrequencyHz: 24000
        },
        chat: {
          minVersion: 1,
          maxVersion: 1
        },
        control: {
          minVersion: 1,
          maxVersion: 3
        },
        input: {
          minVersion: 1,
          maxVersion: 8
        },
        message: {
          minVersion: 1,
          maxVersion: 1
        }
      }
    })
  }

  async sendChatSdp(sessionId: string, sdp: string): Promise<void> {
    const path = `${this.sessionBasePath}/${sessionId}/sdp`
    await this.httpClient.requestJson('POST', path, {
      messageType: 'offer',
      sdp,
      configuration: {
        isMediaStreamsChatRenegotiation: true
      }
    })
  }

  async getSdpExchangeResponse(sessionId: string): Promise<StreamingAnswerPayload | null> {
    const path = `${this.sessionBasePath}/${sessionId}/sdp`
    const result = await this.httpClient.requestJson<StreamingExchangeResponse>('GET', path)
    if (typeof result.exchangeResponse !== 'string' || result.exchangeResponse.length === 0) {
      return null
    }

    const payload = JSON.parse(result.exchangeResponse) as Partial<StreamingAnswerPayload>
    if (typeof payload.sdp !== 'string' || payload.sdp.length === 0) {
      console.warn('[Streaming][ExchangeOffer] answer payload missing sdp', {
        sessionId,
        exchangeResponse: result.exchangeResponse.slice(0, 500)
      })
      throw new Error('Streaming answer SDP is missing.')
    }

    return {
      sdp: payload.sdp,
      messageType: typeof payload.messageType === 'string' ? payload.messageType : undefined
    }
  }

  async getIceExchangeResponse(sessionId: string): Promise<StreamingIceCandidate[] | null> {
    const path = `${this.sessionBasePath}/${sessionId}/ice`
    const result = await this.httpClient.requestJson<StreamingExchangeResponse | string>('GET', path)
    if (result === '') {
      return null
    }

    if (typeof result === 'string') {
      try {
        const parsed = JSON.parse(result) as StreamingIceCandidate[]
        return this.iceNormalizer.normalize(parsed)
      } catch {
        return []
      }
    }

    const payload =
      typeof result.exchangeResponse === 'string'
        ? (JSON.parse(result.exchangeResponse) as StreamingIceCandidate[])
        : []
    return this.iceNormalizer.normalize(payload)
  }

  async sendIce(sessionId: string, ice: StreamingIceCandidate[]): Promise<void> {
    const path = `${this.sessionBasePath}/${sessionId}/ice`
    await this.httpClient.requestJson('POST', path, {
      messageType: 'iceCandidate',
      candidate: ice
    })
  }
}
