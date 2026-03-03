import { HttpClient } from '../../auth/infrastructure/clients/http-client'

interface StreamingHttpClientDeps {
  host: string
  bearerToken: string
  httpClient?: HttpClient
}

function parseJsonOrText<T>(value: string): T | string {
  try {
    return JSON.parse(value) as T
  } catch {
    return value
  }
}

// 统一封装串流域 HTTP 访问，避免 session/signaling API 各自处理认证头和解析。
export class StreamingHttpClient {
  private readonly host: string
  private readonly bearerToken: string
  private readonly httpClient: HttpClient

  constructor(deps: StreamingHttpClientDeps) {
    this.host = deps.host
    this.bearerToken = deps.bearerToken
    this.httpClient = deps.httpClient ?? new HttpClient()
  }

  async requestJson<T = unknown>(
    method: 'GET' | 'POST' | 'DELETE',
    path: string,
    body?: string | Record<string, unknown>,
    extraHeaders: Record<string, string> = {}
  ): Promise<T> {
    const headers = {
      Authorization: `Bearer ${this.bearerToken}`,
      'Content-Type': 'application/json',
      ...extraHeaders
    }

    if (method === 'GET') {
      return (await this.httpClient.get<T>(this.host, path, headers)).body
    }

    if (method === 'POST') {
      return (await this.httpClient.post<T>(this.host, path, headers, body ?? '')).body
    }

    const response = await fetch(`https://${this.host}${path}`, {
      method,
      headers
    })
    const responseText = await response.text()
    if (!response.ok) {
      const error = new Error(`HTTP request failed: ${this.host}${path} (${response.status})`)
      Object.assign(error, {
        status: response.status,
        body: responseText,
        url: path
      })
      throw error
    }

    if (responseText.trim().length === 0) {
      return '' as T
    }

    return parseJsonOrText<T>(responseText) as T
  }

  async fetchJson<T = unknown>(
    path: string,
    extraHeaders: Record<string, string> = {}
  ): Promise<T> {
    const response = await fetch(`https://${this.host}${path}`, {
      method: 'GET',
      headers: {
        Authorization: `Bearer ${this.bearerToken}`,
        'Content-Type': 'application/json',
        ...extraHeaders
      }
    })

    if (!response.ok) {
      throw new Error(`HTTP request failed: ${this.host}${path} (${response.status})`)
    }

    return (await response.json()) as T
  }
}
