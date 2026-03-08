import type { BaseChannel, ChannelContext } from '../../protocol/channels/BaseChannel'
import type { WebRtcTransport } from './WebRtcTransport'

export class DataChannelHub {
  private readonly handlers = new Map<string, BaseChannel>()

  constructor(private readonly transport: WebRtcTransport) {}

  register(name: string, init: RTCDataChannelInit, create: (context: ChannelContext) => BaseChannel): BaseChannel {
    const channel = this.transport.createDataChannel(name, init)
    const context: ChannelContext = {
      send: (data) => {
        if (typeof data === 'string') {
          channel.send(data)
        }
        else {
          channel.send(data)
        }
      },
      readyState: () => channel.readyState,
    }
    const handler = create(context)
    this.handlers.set(name, handler)
    channel.addEventListener('open', () => handler.onOpen())
    channel.addEventListener('message', event => handler.onMessage(event))
    channel.addEventListener('closing', () => handler.onClosing())
    channel.addEventListener('close', () => handler.onClose())
    channel.addEventListener('error', () => handler.onError())
    return handler
  }

  get<T extends BaseChannel>(name: string): T | undefined {
    return this.handlers.get(name) as T | undefined
  }

  clear(): void {
    this.handlers.clear()
  }
}
