import type { ServerMsg } from '../../shared/protocol';

/** A client connection: reliable control messages plus unreliable datagrams. */
export interface Conn {
  readonly kind: 'ws' | 'wt';
  readonly ip: string;
  send(msg: ServerMsg): void;
  datagram(data: Uint8Array): void;
  close(reason?: string): void;
}
