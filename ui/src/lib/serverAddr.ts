// `[server]` holds one `url` ("host:port"); the old `host`/`port` pair is read
// only when `url` is absent (engine `ServerConfig::addr`). The forms read and
// write the address through here so an edit never lands in a field `url` hides.

import type { AppConfig } from '../types';

type Server = AppConfig['server'];

export const DEFAULT_HOST = '127.0.0.1';
export const DEFAULT_PORT = 9527;

export function serverAddr(s: Server): { host: string; port: number } {
  const raw = s.url?.trim();
  if (raw) {
    const rest = (raw.split('://')[1] ?? raw).replace(/\/+$/, '');
    const at = rest.lastIndexOf(':');
    const port = Number(rest.slice(at + 1));
    if (at > 0 && Number.isInteger(port)) return { host: rest.slice(0, at), port };
  }
  return { host: s.host || DEFAULT_HOST, port: s.port || DEFAULT_PORT };
}

/** The server block with its address set — written as `url`, legacy pair dropped. */
export function withServerAddr(s: Server, patch: { host?: string; port?: number }): Server {
  const cur = serverAddr(s);
  const host = patch.host ?? cur.host;
  const port = patch.port ?? cur.port;
  return { ...s, url: `${host}:${port}`, host: undefined, port: undefined };
}
