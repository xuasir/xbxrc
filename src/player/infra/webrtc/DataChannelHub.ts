import type { BaseChannel, ChannelContext } from '../../protocol/channels/BaseChannel'
import type { WebRtcTransport } from './WebRtcTransport'

export class DataChannelHub {
  private readonly handlers = new Map<string, BaseChannel>()

  constructor(private readonly transport: WebRtcTransport) {}

  register(name: string, init: RTCDataChannelInit, create: (context: ChannelContext) => BaseChannel): BaseChannel {
    const channel = this.transport.createDataChannel(name, init)
    const context: ChannelContext = {
      send: (data) => {
        try {
          if (typeof data === 'string') {
            channel.send(data)
          }
          else {
            channel.send(data)
          }
          return true
        }
        catch (error) {
          // 某些浏览器在 readyState 仍显示 open 时也会因底层 SCTP 状态异常而直接抛错。
          // 这里统一吞掉瞬时发送异常，避免控制台被周期性 keyframe 请求刷爆。
          console.warn('[player][data-channel] send failed', {
            label: name,
            readyState: channel.readyState,
            error: error instanceof Error ? error.message : String(error),
          })
          return false
        }
      },
      readyState: () => channel.readyState,
      bufferedAmount: () => channel.bufferedAmount,
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
