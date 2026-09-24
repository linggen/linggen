/**
 * Pet-only surface — the desktop shell's transparent pet window loads this.
 *
 * Renders just Yinyue's avatar on a transparent background. The WebRTC
 * transport + event handlers are hoisted to Root (entries/main.tsx), so
 * pet_speak / pet_express / voice all flow in exactly as they do for the
 * in-page overlay — no separate connection, no duplicated renderer.
 *
 * Selected by `?pet=1`. The native shell points its always-on-top pet window at
 * `http://127.0.0.1:9527/?pet=1` (linggen-app shell/src/pet.rs), so the app and
 * the browser render the SAME core renderer — one Yinyue, no fork.
 */
import React, { useEffect } from 'react';
import { YinyueAvatar } from '../components/yinyue/YinyueAvatar';
import { YinyueBubble } from '../components/YinyueBubble';
import { setYinyueStageSession, useYinyuePresenter } from '../hooks/useTransport';
import { fromParent, postToParent } from '../lib/parentFrame';

/** `?pet=1&stage=1`: this surface is a stage (a scene that stands her in a place). */
const isStage = new URLSearchParams(window.location.search).get('stage') === '1';

/** A stage learns its page's app chat session from the page (the chat bridge
 *  answers `which`, and tells again whenever the chat's session changes). */
function useStageChatSession(): void {
  useEffect(() => {
    if (!isStage) return;
    const onMessage = (e: MessageEvent) => {
      if (!fromParent(e) || e.data?.type !== 'linggen-app-chat' || e.data.event) return;
      setYinyueStageSession(typeof e.data.session === 'string' ? e.data.session : null);
    };
    window.addEventListener('message', onMessage);
    postToParent({ type: 'linggen-app-chat', event: 'which' });
    return () => window.removeEventListener('message', onMessage);
  }, []);
}

export const PetApp: React.FC = () => {
  useStageChatSession();
  // Subscribe to the server's presenter lock — one device, one voice. The pet
  // window normally holds her (it opens first / stays open); a page that stands
  // her in a place loads this same view with `&stage=1` and outranks the corner,
  // so she walks onto that stage and comes back when it closes.
  const showYinyue = useYinyuePresenter(true, isStage);
  // She rides in a transparent always-on-top window; keep the page see-through
  // so only her body paints (the WebGL canvas already clears with alpha).
  useEffect(() => {
    const html = document.documentElement;
    const { body } = document;
    const prev = { html: html.style.background, body: body.style.background };
    html.style.background = 'transparent';
    body.style.background = 'transparent';
    return () => {
      html.style.background = prev.html;
      body.style.background = prev.body;
    };
  }, []);

  return (
    <div className="fixed inset-0 bg-transparent">
      {showYinyue && <YinyueAvatar />}
      {/* Her reply text — without this the pet window shows no response. */}
      {showYinyue && <YinyueBubble variant="pet" />}
    </div>
  );
};
