export interface Logger {
  info(msg: string, data?: Record<string, unknown>): void;
  warn(msg: string, data?: Record<string, unknown>): void;
}

export const NOOP_LOG: Logger = { info() {}, warn() {} };

export function createLogger(pretty: boolean): Logger {
  const out = (level: string, msg: string, data?: Record<string, unknown>) => {
    if (pretty) {
      const extra = data ? ` ${JSON.stringify(data)}` : '';
      console.log(`${new Date().toISOString().slice(11, 19)} ${level === 'warn' ? '⚠' : '·'} ${msg}${extra}`);
    } else console.log(JSON.stringify({ ts: new Date().toISOString(), level, msg, ...data }));
  };
  return { info: (m, d) => out('info', m, d), warn: (m, d) => out('warn', m, d) };
}
