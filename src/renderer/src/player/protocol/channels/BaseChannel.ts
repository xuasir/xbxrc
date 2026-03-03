export interface ChannelContext {
  send(data: ArrayBuffer | string): void;
  readyState(): RTCDataChannelState;
}

export abstract class BaseChannel {
    constructor(protected readonly context: ChannelContext) {}

    onOpen(): void {}
    onClose(): void {}
    onClosing(): void {}
    onError(): void {}
  abstract onMessage(event: MessageEvent): void;

  protected send(data: ArrayBuffer | string): void {
      if (this.context.readyState() !== 'open') {
          return
      }
      if (typeof data === 'string') {
          this.context.send(new TextEncoder().encode(data).buffer)
          return
      }
      this.context.send(data)
  }
}
