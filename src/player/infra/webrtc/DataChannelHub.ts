import type { BaseChannel, ChannelContext } from '../../protocol/channels/BaseChannel'
import type { WebRtcTransport } from './WebRtcTransport'
import { devWarnRateLimited } from '../../../shared/dev-log'

export class DataChannelHub {
  private readonly handlers = new Map<string, BaseChannel>()
  private readonly disposers = new Map<string, () => void>()

  constructor(private readonly transport: WebRtcTransport) {}

  register(name: string, init: RTCDataChannelInit, create: (context: ChannelContext) => BaseChannel): BaseChannel {
    this.disposers.get(name)?.()
    const channel = this.transport.createDataChannel(name, init)
    let active = true
    const writableDisposers = new Set<() => void>()
    const context: ChannelContext = {
      send: (data) => {
        if (!active)
          return false
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
          devWarnRateLimited(`data-channel:send-failed:${name}`, '[player][data-channel] send failed', {
            label: name,
            readyState: channel.readyState,
            error: error instanceof Error ? error.message : String(error),
          })
          return false
        }
      },
      readyState: () => active ? channel.readyState : 'closed',
      bufferedAmount: () => channel.bufferedAmount,
      onWritable: (threshold, listener) => {
        channel.bufferedAmountLowThreshold = threshold
        const onLow = () => {
          if (active)
            listener()
        }
        const unsubscribe = () => {
          channel.removeEventListener('bufferedamountlow', onLow)
          writableDisposers.delete(unsubscribe)
        }
        if (active) {
          channel.addEventListener('bufferedamountlow', onLow)
          writableDisposers.add(unsubscribe)
        }
        return unsubscribe
      },
    }
    const handler = create(context)
    const onOpen = () => {
      if (active)
        handler.onOpen()
    }
    const onMessage = (event: MessageEvent) => {
      if (active)
        handler.onMessage(event)
    }
    const onClosing = () => {
      if (active)
        handler.onClosing()
    }
    const onError = () => {
      if (active)
        handler.onError()
    }
    const dispose = () => {
      if (!active)
        return
      // 先撤销旧通道事件，再回收共享输入；迟到 close/握手只属于原会话。
      active = false
      for (const unsubscribe of writableDisposers)
        unsubscribe()
      channel.removeEventListener('open', onOpen)
      channel.removeEventListener('message', onMessage)
      channel.removeEventListener('closing', onClosing)
      channel.removeEventListener('close', dispose)
      channel.removeEventListener('error', onError)
      handler.onClose()
      channel.close()
    }
    this.handlers.set(name, handler)
    this.disposers.set(name, dispose)
    channel.addEventListener('open', onOpen)
    channel.addEventListener('message', onMessage)
    channel.addEventListener('closing', onClosing)
    channel.addEventListener('close', dispose)
    channel.addEventListener('error', onError)
    return handler
  }

  get<T extends BaseChannel>(name: string): T | undefined {
    return this.handlers.get(name) as T | undefined
  }

  clear(): void {
    for (const dispose of this.disposers.values())
      dispose()
    this.disposers.clear()
    this.handlers.clear()
  }
}
