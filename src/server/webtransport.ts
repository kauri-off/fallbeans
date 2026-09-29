import { createServer, type ServerSession, type WebTransportServer } from '@webtransport-bun/webtransport';
import { FrameReader, frame } from '../shared/codec';
import { MAX_CONTROL_BYTES, type ServerMsg } from '../shared/protocol';
import type { Gateway } from './gateway';
import type { Conn, Logger } from './room';

const enc = new TextEncoder();
const dec = new TextDecoder();

/**
 * WebTransport (HTTP/3 over UDP): one client-opened bidirectional stream carries length-prefixed
 * JSON control messages; datagrams carry inputs and snapshots.
 */
export function startWebTransport(opts: {
  port: number;
  certPem: string;
  keyPem: string;
  gateway: Gateway;
  log: Logger;
}): WebTransportServer {
  const { gateway, log } = opts;
  return createServer({
    host: '0.0.0.0',
    port: opts.port,
    tls: { certPem: opts.certPem, keyPem: opts.keyPem },
    limits: { maxSessions: 64, maxStreamsPerSessionBidi: 2, maxStreamsPerSessionUni: 2, idleTimeoutMs: 30_000 },
    rateLimits: {
      handshakesPerSec: 10,
      handshakesBurst: 20,
      datagramsPerSec: 400,
      datagramsBurst: 800,
      streamsPerSec: 10,
      streamsBurst: 20,
    },
    log: (e) => {
      if (e.level === 'warn' || e.level === 'error') log.warn(`webtransport: ${e.msg}`, { peer: e.peerIp, ...e.data });
    },
    onSession: (s) => handle(s, gateway, log),
  });
}

async function handle(s: ServerSession, gateway: Gateway, log: Logger) {
  const pending: Uint8Array[] = [];
  let writer: WritableStreamDefaultWriter<Uint8Array> | null = null;
  let closed = false;
  const write = (bytes: Uint8Array) => {
    if (closed) return;
    if (!writer) {
      pending.push(bytes);
      return;
    }
    writer.write(bytes).catch(() => {});
  };
  const conn: Conn = {
    kind: 'wt',
    ip: s.peer.ip,
    send: (msg: ServerMsg) => write(frame(enc.encode(JSON.stringify(msg)))),
    datagram: (data) => {
      if (!closed) s.sendDatagram(data).catch(() => {});
    },
    close: () => {
      if (closed) return;
      closed = true;
      s.close({ code: 0, reason: 'bye' });
    },
  };
  const session = gateway.open(conn);
  s.closed.then(() => {
    closed = true;
    session.close();
  });

  (async () => {
    try {
      for await (const d of s.incomingDatagrams()) session.datagram(d);
    } catch {}
  })();

  try {
    const streams = s.incomingBidirectionalStreams.getReader();
    const first = await streams.read();
    streams.releaseLock();
    if (first.done || !first.value) return;
    writer = first.value.writable.getWriter();
    for (const b of pending.splice(0)) writer.write(b).catch(() => {});
    const reader = first.value.readable.getReader();
    const frames = new FrameReader(MAX_CONTROL_BYTES);
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      for (const f of frames.push(value)) session.control(dec.decode(f));
    }
  } catch (e) {
    if (!closed) log.warn('webtransport stream error', { ip: s.peer.ip, err: String(e) });
  } finally {
    conn.close();
  }
}
