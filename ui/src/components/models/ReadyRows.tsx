/**
 * 开箱即用: the Linggen Cloud account and the ChatGPT subscription. A sign-in
 * button and the usage the engine already reports; no key field. ChatGPT lists
 * its built-in models under its name; Linggen Cloud opens to its models to
 * tick (`CloudList`), then its live voice and read-aloud, ticked and fixed.
 */
import React, { useState } from 'react';
import { LogIn, LogOut } from 'lucide-react';
import { account as accountApi, modelsApi, providerAuth } from '../../lib/endpoints';
import { W } from '../../lib/modelsWords.mts';
import type { ModelMove } from '../../types';
import type { Provider } from '../../lib/providers.mts';
import { CodexUsage } from '../CodexUsage';
import { UsageLink, useCloudUsage, usageToneCls } from '../UsageMeter';
import { CloudList } from './CloudList';
import { Row, sectionLabel, sectionRule } from './shell';
import type { Run } from './useProviders';

const CODEX = { login: '/api/auth/codex/login', logout: '/api/auth/codex/logout', status: '/api/auth/codex/status' };
const signInBtn = 'flex items-center gap-1.5 rounded-full bg-blue-600 px-2.5 py-1 text-[11px] font-semibold text-white hover:bg-blue-700 disabled:opacity-50';

type Props = {
  provider: Provider;
  /** The engine's note on each built-in whose family moved (ChatGPT only). */
  moved: Record<string, ModelMove | undefined>;
  /** Read the page again after a sign-in, sign-out or stay. */
  changed: () => Promise<void>;
  /** What the opened Linggen Cloud row needs to tick a model (as a key row has it). */
  run: Run;
  busy: boolean;
  makeDefault: (modelId: string) => Promise<void>;
  open: boolean;
  onToggle: () => void;
};


/** Where ChatGPT's sign-in stands, then its built-in model ids. */
const subtitle = (p: Provider, note: string) => (
  <>
    <span>{p.fix === 'sign_in' ? W.notSignedIn : W.signedIn}</span>
    <span aria-hidden> · </span>
    <span>{note}</span>
    <span aria-hidden> · </span>
    <span data-no-i18n>{p.models.filter((m) => m.builtin && m.kind === 'chat').map((m) => m.id).join(' · ')}</span>
  </>
);

export const ReadyRow: React.FC<Props> = (props) => {
  const { provider: p } = props;
  if (p.way === 'account') return <CloudRow {...props} />;
  if (p.provider === 'chatgpt') return <ChatgptRow {...props} />;
  return <Row testId={p.provider} name={p.label} way={p.way} subtitle={subtitle(p, '')} />;
};

const CloudRow: React.FC<Props> = ({ provider: p, changed, run, busy, open, onToggle }) => {
  const [waiting, setWaiting] = useState(false);
  const signedIn = p.fix !== 'sign_in';
  const usage = useCloudUsage(signedIn);
  const signIn = async () => {
    setWaiting(true);
    try {
      if (await accountApi.signIn()) await changed();
    } finally {
      setWaiting(false);
    }
  };
  const right = signedIn ? undefined
    : <button type="button" className={signInBtn} disabled={waiting} onClick={() => void signIn()}><LogIn size={12} />{waiting ? W.waitingBrowser : W.signInLinggen}</button>;
  // Collapsed: whether the account is signed in, and how much of the allowance is used.
  const line = (
    <>
      <span>{signedIn ? W.signedIn : W.notSignedIn}</span>
      {usage && <><span aria-hidden> · </span><span data-testid="cloud-usage" className={usageToneCls(usage.tone)}>{usage.text}</span></>}
    </>
  );
  return (
    <Row testId="linggen-cloud" name={p.label} way={p.way} subtitle={line} right={right} open={open} onToggle={onToggle}>
      <div className={`${sectionRule} grid gap-2 sm:grid-cols-[4rem_minmax(0,1fr)]`}>
        <span className={sectionLabel}>{W.models}</span>
        <div className="min-w-0 flex-1">
          <CloudList provider={p} run={run} busy={busy} />
          {usage?.metered && <p className="mt-1"><UsageLink usage={usage} /></p>}
        </div>
      </div>
    </Row>
  );
};

const ChatgptRow: React.FC<Props> = ({ provider: p, moved, changed }) => {
  const [waiting, setWaiting] = useState(false);
  const signedIn = p.fix !== 'sign_in';
  const signIn = async () => {
    setWaiting(true);
    try {
      await providerAuth.login(CODEX.login);
      // The browser login takes as long as the person takes: look every two seconds, for five minutes.
      for (let waited = 0; waited < 300_000; waited += 2000) {
        await new Promise((r) => setTimeout(r, 2000));
        const status = await providerAuth.status<{ authenticated: boolean }>(CODEX.status).catch(() => null);
        if (status?.authenticated) { await changed(); return; }
      }
    } finally {
      setWaiting(false);
    }
  };
  const signOut = async () => {
    await providerAuth.logout(CODEX.logout).catch(() => {});
    await changed();
  };
  const right = signedIn ? (
    <>
      <CodexUsage />
      <button type="button" className="flex items-center gap-1 font-medium text-red-500 hover:text-red-600" onClick={() => void signOut()}><LogOut size={10} />{W.signOut}</button>
    </>
  ) : (
    <button type="button" className={signInBtn} disabled={waiting} onClick={() => void signIn()}><LogIn size={12} />{waiting ? W.waitingBrowser : W.signInChatgpt}</button>
  );
  const moves = p.models.filter((m) => moved[m.id]);
  return (
    <div>
      <Row testId="chatgpt" name={p.label} way={p.way} subtitle={subtitle(p, W.chatgptNote)} right={right} />
      {moves.map((m) => <FamilyMoved key={m.id} move={moved[m.id]!} onChanged={changed} />)}
    </div>
  );
};

/** One line on a ChatGPT built-in whose family moved — to what, since when —
 *  and the one control to stay on the previous model or follow again. */
function FamilyMoved({ move, onChanged }: { move: ModelMove; onChanged: () => Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const date = move.since ? new Date(move.since * 1000).toLocaleDateString() : '';
  const toggle = async () => {
    setBusy(true);
    try {
      await modelsApi.stayChatgpt(move.family, !move.staying);
      await onChanged();
    } finally {
      setBusy(false);
    }
  };
  const line = move.staying
    ? `Staying on ${move.previous}; ${move.current} is newer.`
    : `Moved up from ${move.previous} on ${date}.`;
  const action = move.staying ? `Use ${move.current}` : `Stay on ${move.previous}`;
  return (
    <p className="mx-3 mt-1 text-[11px] text-slate-500">
      <span>{line}</span>
      {(move.staying || move.can_stay) && (
        <button type="button" onClick={() => void toggle()} disabled={busy} className="ml-2 text-[11px] font-semibold text-blue-600 hover:text-blue-700 disabled:opacity-50">
          {action}
        </button>
      )}
    </p>
  );
}
