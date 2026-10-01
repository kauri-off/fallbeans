import { FrameReader, frame } from '../../shared/codec';
import { BASE_PATH } from '../../shared/consts';
import { MAX_CONTROL_BYTES, type ServerMsg } from '../../shared/protocol';

export interface SessionInfo {
  ticket: string;
  version: number;
  /** The server's build (null in development): a page of another build reloads. */
  build?: string | null;
  wt: { port: number; hashes: string[] } | null;
}

export interface TransportEvents {
  control(msg: ServerMsg): void;
  datagram(data: Uint8Array): void;
  close(reason: string): void;
}

export interface Transport {
  readonly kind: 'wt' | 'ws';
  sendControl(msg: object): void;
  sendDatagram(data: Uint8Array<ArrayBuffer>): void;
  close(): void;
}

const enc = new TextEncoder();
const dec = new TextDecoder();

function b64ToBytes(b64: string): Uint8Array<ArrayBuffer> {
  const s = atob(b64);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

async function connectWebTransport(info: NonNullable<SessionInfo['wt']>, ev: TransportEvents): Promise<Transport> {
  // "localhost" may resolve to ::1 while the dev server listens on IPv4.
  const host = location.hostname === 'localhost' ? '127.0.0.1' : location.hostname;
  const url = `https://${host}:${info.port}${BASE_PATH}wt`;
  const opts: WebTransportOptions = {};
  if (info.hashes.length)
    opts.serverCertificateHashes = info.hashes.map((h) => ({ algorithm: 'sha-256' as const, value: b64ToBytes(h) }));
  const wt = new WebTransport(url, opts);
  const timeout = new Promise<never>((_, rej) => setTimeout(() => rej(new Error('timeout')), 3000));
  await Promise.race([wt.ready, timeout]);
  const stream = await wt.createBidirectionalStream();
  // Datagrams that could not leave for a while (a congested link) are stale: dropped rather than
  // queued (every input packet repeats the inputs not yet acknowledged anyway).
  try {
    const dg = wt.datagrams as unknown as { outgoingMaxAge?: number | null; incomingMaxAge?: number | null };
    dg.outgoingMaxAge = 150;
    dg.incomingMaxAge = 250;
  } catch {}
  const writer = stream.writable.getWriter();
  const dgWriter = wt.datagrams.writable.getWriter();
  let closed = false;
  const finish = (reason: string) => {
    if (closed) return;
    closed = true;
    ev.close(reason);
  };
  wt.closed.then(
    () => finish('closed'),
    () => finish('error'),
  );
  void (async () => {
    const reader = stream.readable.getReader();
    const frames = new FrameReader(1 << 20);
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        for (const f of frames.push(value)) ev.control(JSON.parse(dec.decode(f)) as ServerMsg);
      }
    } catch {}
    finish('stream');
  })();
  void (async () => {
    const reader = wt.datagrams.readable.getReader();
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        ev.datagram(value);
      }
    } catch {}
  })();
  return {
    kind: 'wt',
    sendControl(msg) {
      const bytes = enc.encode(JSON.stringify(msg));
      if (bytes.length <= MAX_CONTROL_BYTES) writer.write(frame(bytes)).catch(() => {});
    },
    sendDatagram(data) {
      dgWriter.write(data).catch(() => {});
    },
    close() {
      closed = true;
      try {
        wt.close();
      } catch {}
    },
  };
}

function connectWebSocket(ev: TransportEvents): Promise<Transport> {
  return new Promise((resolve, reject) => {
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${proto}://${location.host}${BASE_PATH}ws`);
    ws.binaryType = 'arraybuffer';
    let open = false;
    ws.onopen = () => {
      open = true;
      resolve({
        kind: 'ws',
        sendControl(msg) {
          if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
        },
        sendDatagram(data) {
          // Behind on a slow link (TCP waiting on a lost packet): drop, like a lost datagram; the
          // next packet repeats every input not yet acknowledged. A deep queue would only add lag.
          if (ws.readyState === WebSocket.OPEN && ws.bufferedAmount < 2 * 1024) ws.send(data);
        },
        close() {
          ws.close();
        },
      });
    };
    ws.onmessage = (e) => {
      if (typeof e.data === 'string') ev.control(JSON.parse(e.data) as ServerMsg);
      else ev.datagram(new Uint8Array(e.data as ArrayBuffer));
    };
    ws.onclose = () => {
      if (open) ev.close('closed');
      else reject(new Error('websocket failed'));
    };
  });
}

/** WebTransport first (datagrams over QUIC); WebSocket when it is unavailable or blocked. */
export async function connect(info: SessionInfo, ev: TransportEvents, preferWs = false): Promise<Transport> {
  if (!preferWs && info.wt && typeof WebTransport !== 'undefined') {
    try {
      return await connectWebTransport(info.wt, ev);
    } catch (e) {
      console.warn('WebTransport unavailable, using WebSocket', e);
    }
  }
  return connectWebSocket(ev);
}

export async function fetchSession(): Promise<SessionInfo | 'error' | 'updating'> {
  try {
    const r = await fetch(`${BASE_PATH}api/session`, { cache: 'no-store', credentials: 'same-origin' });
    if (r.status === 503) {
      const body = (await r.json().catch(() => null)) as { updating?: boolean } | null;
      if (body?.updating) return 'updating';
    }
    if (!r.ok) return 'error';
    return (await r.json()) as SessionInfo;
  } catch {
    return 'error';
  }
}
