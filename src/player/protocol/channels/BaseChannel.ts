export interface ChannelContext {
  send: (data: ArrayBuffer | string) => boolean
  readyState: () => RTCDataChannelState
  bufferedAmount: () => number
}

export abstract class BaseChannel {
  constructor(protected readonly context: ChannelContext) {}

  onOpen(): void {
    return undefined
  }

  onClose(): void {
    return undefined
  }

  onClosing(): void {
    return undefined
  }

  onError(): void {
    return undefined
  }
  abstract onMessage(event: MessageEvent): void

  protected send(data: ArrayBuffer | string): boolean {
    if (this.context.readyState() !== 'open') {
      return false
    }
    return this.context.send(data)
  }
}
