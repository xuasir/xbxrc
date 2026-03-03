import { IceCandidateLike, SessionState } from '../domain/session'
import { ProcessedVideoFrameMetadata } from '../domain/input'
import { StreamStats } from '../domain/media'
import { ChannelMessage } from '../domain/messages'
import { FpsStats, InputPacketStats } from '../domain/stats'

export interface PlayerEvents {
  'session.stateChanged': { from: SessionState; to: SessionState };
  'transport.iceCandidate': IceCandidateLike;
  'transport.connectionState': { state: RTCPeerConnectionState };
  'transport.track': { kind: 'audio' | 'video'; stream: MediaStream };
  'channel.message': ChannelMessage;
  'stats.updated': StreamStats;
  'stats.fps': FpsStats;
  'stats.inputPacket': InputPacketStats;
  'stats.videoFrameProcessed': ProcessedVideoFrameMetadata;
  'media.videoReady': { width: number; height: number };
  'media.audioReady': Record<string, never>;
  'chat.stateChanged': { capturing: boolean; paused: boolean };
  'error': { error: unknown };
}

type Listener<T> = (payload: T) => void

export class TypedEventEmitter<TEvents extends Record<string, any>> {
    private listeners = new Map<keyof TEvents, Set<Listener<any>>>()

    on<K extends keyof TEvents>(event: K, listener: Listener<TEvents[K]>): () => void {
        let bucket = this.listeners.get(event)
        if (!bucket) {
            bucket = new Set()
            this.listeners.set(event, bucket)
        }
        bucket.add(listener)
        return () => bucket?.delete(listener)
    }

    emit<K extends keyof TEvents>(event: K, payload: TEvents[K]): void {
        const bucket = this.listeners.get(event)
        if (!bucket) {
            return
        }
        for (const listener of bucket) {
            listener(payload)
        }
    }

    clear(): void {
        this.listeners.clear()
    }
}
