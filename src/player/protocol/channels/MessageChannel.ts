import type { PlayerEvents, TypedEventEmitter } from '../../api/events'
import type { ChannelContext } from './BaseChannel'
import {
  STREAM_DEFAULT_VIEWPORT,
  STREAM_MESSAGE_HANDSHAKE,
} from '../networkProfile'
import { BaseChannel } from './BaseChannel'

export interface MessageChannelOptions {
  uiVersion: Array<number>
  uiSystem: Array<number>
}

export interface MessageChannelDelegate {
  onHandshakeAck: () => void
}

type MessagePayload = Record<string, unknown>

type MessageEnvelope = {
  type?: string
  content?: string
} & Record<string, unknown>

type WindowWithXboxTitle = Window & {
  _xboxTitleId?: number
}

function parseJsonMessage(value: string): MessageEnvelope {
  return JSON.parse(value) as MessageEnvelope
}

export class MessageChannel extends BaseChannel {
  constructor(
    context: ChannelContext,
    private readonly options: MessageChannelOptions,
    private readonly delegate: MessageChannelDelegate,
    private readonly emitter: TypedEventEmitter<PlayerEvents>,
  ) {
    super(context)
  }

  onOpen(): void {
    console.info('[player][message] open')
    this.send(
      JSON.stringify({
        type: 'Handshake',
        version: STREAM_MESSAGE_HANDSHAKE.version,
        id: STREAM_MESSAGE_HANDSHAKE.id,
        cv: '',
      }),
    )
  }

  onMessage(event: MessageEvent): void {
    if (typeof event.data !== 'string') {
      return
    }
    const jsonMessage = parseJsonMessage(event.data)
    if (jsonMessage.type === 'HandshakeAck') {
      console.info('[player][message] HandshakeAck received')
      this.delegate.onHandshakeAck()
      this.send(
        JSON.stringify(
          this.generateMessage('/streaming/systemUi/configuration', {
            version: this.options.uiVersion,
            systemUis: this.options.uiSystem,
          }),
        ),
      )
      this.send(
        JSON.stringify(
          this.generateMessage('/streaming/properties/clientappinstallidchanged', {
            clientAppInstallId: STREAM_MESSAGE_HANDSHAKE.clientAppInstallId,
          }),
        ),
      )
      this.send(
        JSON.stringify(
          this.generateMessage('/streaming/characteristics/orientationchanged', {
            orientation: 0,
          }),
        ),
      )
      this.send(
        JSON.stringify(
          this.generateMessage('/streaming/characteristics/touchinputenabledchanged', {
            touchInputEnabled: false,
          }),
        ),
      )
      this.send(
        JSON.stringify(
          this.generateMessage('/streaming/characteristics/clientdevicecapabilities', {}),
        ),
      )
      this.send(
        JSON.stringify(
          this.generateMessage('/streaming/characteristics/dimensionschanged', {
            horizontal: STREAM_DEFAULT_VIEWPORT.width,
            vertical: STREAM_DEFAULT_VIEWPORT.height,
            preferredWidth: STREAM_DEFAULT_VIEWPORT.width,
            preferredHeight: STREAM_DEFAULT_VIEWPORT.height,
            safeAreaLeft: 0,
            safeAreaTop: 0,
            safeAreaRight: STREAM_DEFAULT_VIEWPORT.width,
            safeAreaBottom: STREAM_DEFAULT_VIEWPORT.height,
            supportsCustomResolution: true,
          }),
        ),
      )
    }
    if (event.data.includes('/titleinfo') && typeof jsonMessage.content === 'string') {
      const content = JSON.parse(jsonMessage.content) as { titleid?: string }
      if (typeof content.titleid === 'string') {
        ;(window as WindowWithXboxTitle)._xboxTitleId = Number.parseInt(content.titleid, 16)
      }
    }
    this.emitter.emit('channel.message', jsonMessage)
  }

  private generateMessage(target: string, data: MessagePayload): MessagePayload {
    return {
      type: 'Message',
      content: JSON.stringify(data),
      id: STREAM_MESSAGE_HANDSHAKE.transactionId,
      target,
      cv: '',
    }
  }

  sendTransaction(id: string, data: MessagePayload): void {
    this.send(
      JSON.stringify({
        type: 'TransactionComplete',
        content: JSON.stringify(data),
        id,
        cv: '',
      }),
    )
  }
}
