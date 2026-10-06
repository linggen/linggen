/**
 * The UI's data channel, offered to the skill pages it frames.
 *
 * A skill page has no channel of its own when the UI frames it (the launcher's
 * app tabs, a settings page): its /shared/channel.js finds this host on the
 * frame above and asks it, so the page's `/api/*` calls ride this page's peer
 * — no second peer, never plain HTTP. A page standing alone opens the UI's
 * bare channel view (`/?channel=1`, `startChannelView`) in a hidden frame and
 * asks that instead. doc/webrtc-spec.md § Skill pages.
 */
import { connection, getTransport, setTransport } from './transport';
import { RtcTransport } from './rtcTransport';
import { channelBody, NotConnectedError, ridesChannel, type ProxyReply } from './channelFetch.mts';

/** One call a framed page asks for (shared/channel.js `request`). */
export interface ChannelRequest {
  method: string;
  path: string;
  body?: string;
  contentType?: string;
  /** false: fail at once when the channel is not open (a reading). */
  wait?: boolean;
}

/** What a framed page finds at `window.__linggenChannel`. */
export interface ChannelHost {
  request(req: ChannelRequest): Promise<ProxyReply>;
  isOpen(): boolean;
}

declare global {
  interface Window {
    __linggenChannel?: ChannelHost;
  }
}

/** One framed page's call over this page's data channel — the engine's reply
 *  as it came (a byte body still base64; the page decodes it). */
async function request(req: ChannelRequest): Promise<ProxyReply> {
  const method = req.method.toUpperCase();
  const what = `${method} ${req.path}`;
  if (!ridesChannel(req.path)) throw new Error(`${what}: not a data call`);
  if (req.wait === false && !connection.isOpen()) throw new NotConnectedError(what, 0);
  await connection.whenOpen(what);
  return getTransport().httpProxy(method, req.path, channelBody(req.body, req.contentType));
}

/** Offer this page's channel to the skill pages it frames. */
export function installChannelHost(): void {
  window.__linggenChannel = { request, isOpen: () => connection.isOpen() };
}

/** The bare channel view (`/?channel=1`): a hidden frame a standalone skill
 *  page opens for its calls. One peer, nothing rendered; its view context
 *  (`channel`) tells the engine to push it nothing. */
export function startChannelView(): void {
  const transport = new RtcTransport({ onEvent: () => {}, onStatusChange: () => {} });
  setTransport(transport);
  transport.sendViewContext({ sessionId: null, projectRoot: null, view: 'channel' });
  transport.connect();
}
