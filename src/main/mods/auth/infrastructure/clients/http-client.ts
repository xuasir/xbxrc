export interface HttpResponse<T = unknown> {
  statusCode: number
  headers: Record<string, string>
  body: T
}

export class HttpClient {
  private defaultHeaders: Record<string, string> = {}

  setDefaultHeaders(headers: Record<string, string>): void {
    this.defaultHeaders = headers
  }

  getDefaultHeaders(): Record<string, string> {
    return { ...this.defaultHeaders }
  }

  async get<T = unknown>(
    host: string,
    path: string,
    headers: Record<string, string> = {}
  ): Promise<HttpResponse<T>> {
    return await this.request<T>('GET', host, path, headers)
  }

  async post<T = unknown>(
    host: string,
    path: string,
    headers: Record<string, string> = {},
    data: string | Record<string, unknown> = ''
  ): Promise<HttpResponse<T>> {
    const body = typeof data === 'string' ? data : JSON.stringify(data)
    return await this.request<T>('POST', host, path, headers, body)
  }

  // 统一封装 https 请求，便于在 adapter/client 复用
  private async request<T>(
    method: 'GET' | 'POST',
    host: string,
    path: string,
    headers: Record<string, string>,
    body = ''
  ): Promise<HttpResponse<T>> {
    const requestHeaders = {
      ...headers,
      ...this.defaultHeaders
    }

    const response = await fetch(`https://${host}${path}`, {
      method,
      headers: requestHeaders,
      body: method === 'POST' && body.length > 0 ? body : undefined
    })

    const responseText = await response.text()
    if (!response.ok) {
      throw new Error(`HTTP request failed: ${host}${path} (${response.status}) ${responseText}`)
    }

    let parsedBody: unknown = {}
    if (responseText.trim().length > 0) {
      try {
        parsedBody = JSON.parse(responseText)
      } catch {
        parsedBody = responseText
      }
    }

    const responseHeaders: Record<string, string> = {}
    response.headers.forEach((value, key) => {
      responseHeaders[key] = value
    })

    return {
      statusCode: response.status,
      headers: responseHeaders,
      body: parsedBody as T
    }
  }
}
