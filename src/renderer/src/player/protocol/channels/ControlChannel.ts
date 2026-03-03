import { BaseChannel, ChannelContext } from './BaseChannel'

export interface ControlChannelDelegate {
  onClose(): void;
}

export class ControlChannel extends BaseChannel {
    private keyframeInterval?: number
    private started = false
    private pendingStart = false

    constructor(context: ChannelContext, private readonly delegate: ControlChannelDelegate) {
        super(context)
    }

    onOpen(): void {
        console.info('[player][control] open')
        if (this.pendingStart) {
            this.pendingStart = false
            this.start()
        }
    }

    onMessage(event: MessageEvent): void {
        try {
            JSON.parse(event.data)
        } catch {
            return
        }
    }

    onClose(): void {
        if (this.keyframeInterval) {
            window.clearInterval(this.keyframeInterval)
            this.keyframeInterval = undefined
        }
        this.started = false
        this.pendingStart = false
        this.delegate.onClose()
    }

    sendGamepadAdded(gamepadIndex: number): void {
        this.send(JSON.stringify({ message: 'gamepadChanged', gamepadIndex, wasAdded: true }))
    }

    sendGamepadRemoved(gamepadIndex: number): void {
        this.send(JSON.stringify({ message: 'gamepadChanged', gamepadIndex, wasAdded: false }))
    }

    requestKeyframe(): void {
        this.send(JSON.stringify({ message: 'videoKeyframeRequested', ifrRequested: true }))
    }

    start(): void {
        if (this.started) {
            return
        }
        if (this.context.readyState() !== 'open') {
            console.info('[player][control] start deferred until open')
            this.pendingStart = true
            return
        }
        console.info('[player][control] start authorization flow')
        this.started = true
        this.send(JSON.stringify({
            message: 'authorizationRequest',
            accessKey: '4BDB3609-C1F1-4195-9B37-FEFF45DA8B8E',
        }))
        this.sendGamepadRemoved(0)
        window.setTimeout(() => this.sendGamepadAdded(0), 500)
        this.keyframeInterval = window.setInterval(() => this.requestKeyframe(), 5000)
    }
}
