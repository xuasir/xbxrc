import { TypedEventEmitter } from '../../api/events'
import { BaseChannel, ChannelContext } from './BaseChannel'

export interface MessageChannelOptions {
  uiVersion: Array<number>;
  uiSystem: Array<number>;
  touchEnabled: boolean;
}

export interface MessageChannelDelegate {
  onHandshakeAck(): void;
}

export class MessageChannel extends BaseChannel {
    constructor(
        context: ChannelContext,
    private readonly options: MessageChannelOptions,
    private readonly delegate: MessageChannelDelegate,
    private readonly emitter: TypedEventEmitter<any>,
    ) {
        super(context)
    }

    onOpen(): void {
        console.info('[player][message] open')
        this.send(JSON.stringify({
            type: 'Handshake',
            version: 'messageV1',
            id: 'f9c5f412-0e69-4ede-8e62-92c7f5358c56',
            cv: '',
        }))
    }

    onMessage(event: MessageEvent): void {
        const jsonMessage = JSON.parse(event.data)
        if (jsonMessage.type === 'HandshakeAck') {
            console.info('[player][message] HandshakeAck received')
            this.delegate.onHandshakeAck()
            this.send(JSON.stringify(this.generateMessage('/streaming/systemUi/configuration', {
                version: this.options.uiVersion,
                systemUis: this.options.uiSystem,
            })))
            this.send(JSON.stringify(this.generateMessage('/streaming/properties/clientappinstallidchanged', {
                clientAppInstallId: 'c11ddb2e-c7e3-4f02-a62b-fd5448e0b851',
            })))
            this.send(JSON.stringify(this.generateMessage('/streaming/characteristics/orientationchanged', {
                orientation: 0,
            })))
            this.send(JSON.stringify(this.generateMessage('/streaming/characteristics/touchinputenabledchanged', {
                touchInputEnabled: this.options.touchEnabled,
            })))
            this.send(JSON.stringify(this.generateMessage('/streaming/characteristics/clientdevicecapabilities', {})))
            this.send(JSON.stringify(this.generateMessage('/streaming/characteristics/dimensionschanged', {
                horizontal: 1920,
                vertical: 1080,
                preferredWidth: 1920,
                preferredHeight: 1080,
                safeAreaLeft: 0,
                safeAreaTop: 0,
                safeAreaRight: 1920,
                safeAreaBottom: 1080,
                supportsCustomResolution: true,
            })))
        }
        if (event.data.includes('/titleinfo')) {
            const content = JSON.parse(jsonMessage.content);
            (window as any)._xboxTitleId = parseInt(content.titleid, 16)
        }
        this.emitter.emit('channel.message', jsonMessage)
    }

    private generateMessage(target: string, data: Record<string, any>): Record<string, any> {
        return {
            type: 'Message',
            content: JSON.stringify(data),
            id: '41f93d5a-900f-4d33-b7a1-2d4ca6747072',
            target,
            cv: '',
        }
    }

    sendTransaction(id: string, data: Record<string, any>): void {
        this.send(JSON.stringify({
            type: 'TransactionComplete',
            content: JSON.stringify(data),
            id,
            cv: '',
        }))
    }
}
