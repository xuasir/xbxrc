export interface HandshakeMessage {
  type: 'Handshake';
  version: string;
  id: string;
  cv: string;
}

export interface HandshakeAckMessage {
  type: 'HandshakeAck';
  id?: string;
  cv?: string;
}

export interface ServiceMessage<TContent = Record<string, unknown>> {
  type: 'Message';
  content: string;
  id: string;
  target: string;
  cv: string;
  parsedContent?: TContent;
}

export interface TransactionCompleteMessage<TContent = Record<string, unknown>> {
  type: 'TransactionComplete';
  content: string;
  id: string;
  cv: string;
  parsedContent?: TContent;
}

export type ChannelMessage =
  | HandshakeMessage
  | HandshakeAckMessage
  | ServiceMessage
  | TransactionCompleteMessage
  | Record<string, unknown>
