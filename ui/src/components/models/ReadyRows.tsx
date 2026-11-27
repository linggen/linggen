/**
 * 开箱即用: the Linggen Cloud account. A sign-in button and the usage the
 * engine already reports; no key field. It opens to its models to tick
 * (`CloudList`), then its live voice and read-aloud, ticked and fixed. The
 * ChatGPT sign-in is a provider the person adds (`ChatgptRow`).
 */
import React, { useState } from 'react';
import { LogIn } from 'lucide-react';
import { account as accountApi } from '../../lib/endpoints';
import { W } from '../../lib/modelsWords.mts';
import type { Provider } from '../../lib/providers.mts';
import { UsageLink, useCloudUsage, usageToneCls } from '../UsageMeter';
import { CloudList } from './CloudList';
import { Row, sectionLabel, sectionRule } from './shell';
import type { Run } from './useProviders';

const signInBtn = 'flex items-center gap-1.5 rounded-full bg-blue-600 px-2.5 py-1 text-[11px] font-semibold text-white hover:bg-blue-700 disabled:opacity-50';

type Props = {
  provider: Provider;
  /** Read the page again after a sign-in, sign-out or stay. */
  changed: () => Promise<void>;
  /** What the opened Linggen Cloud row needs to tick a model (as a key row has it). */
  run: Run;
  busy: boolean;
  makeDefault: (modelId: string) => Promise<void>;
  open: boolean;
  onToggle: () => void;
};


export const ReadyRow: React.FC<Props> = (props) => {
  const { provider: p } = props;
  if (p.way === 'account') return <CloudRow {...props} />;
  return null;
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
