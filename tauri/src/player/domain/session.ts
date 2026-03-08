export type SessionState =
  | 'idle'
  | 'binding'
  | 'negotiating'
  | 'connecting'
  | 'connected'
  | 'reconnecting'
  | 'closed'
  | 'failed';

export interface IceCandidateLike {
  candidate: string;
  sdpMid?: string | null;
  sdpMLineIndex?: number | null;
}

export interface TurnServerConfig {
  url: string;
  username?: string;
  credential?: string;
}

export interface ConnectParams {
  turnServer?: TurnServerConfig;
}

export interface CreateOfferOptions {
  iceRestart?: boolean;
}

export interface CodecPreferenceOptions {
  mimeType: string;
  profiles: Array<string>;
}

export interface TransportRuntimeConfig {
  codecPreference?: CodecPreferenceOptions;
  maxVideoBitrateKbps: number;
  maxAudioBitrateKbps: number;
  forceMonoAudio: boolean;
}
