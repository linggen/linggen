/**
 * Merged Linggen settings (app-host). One settings surface for the unified
 * app: shared sections (Account, Model, Yinyue) once at top, then a section
 * per installed app reusing that app's own settings.html (iframed with
 * ?embed=1 so the app hides its now-duplicate Account/Model/pet sections).
 */
import React, { useState, useEffect, useCallback } from 'react';
import { X } from 'lucide-react';

interface AppSkill {
  name: string;
  app: { launcher: string; entry: string };
}

const LABELS: Record<string, string> = {
  cfo: 'CFO',
  'sys-doctor': 'Sys Doctor',
  pulse: 'Pulse',
  dj: 'DJ',
  'shared-memory': 'Memory',
};
const labelFor = (n: string) => LABELS[n] ?? n;

/** Apps that ship a settings.html (others have no per-app settings to show). */
const HAS_SETTINGS = new Set(['cfo', 'sys-doctor', 'pulse', 'dj']);

type Section = { id: string; label: string; kind: 'account' | 'model' | 'yinyue' | 'app'; skill?: AppSkill };

async function bash(command: string): Promise<string> {
  try {
    const r = await fetch('/api/bash', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ project_root: '~', command }),
    });
    const d = await r.json().catch(() => ({}));
    return (d.stdout ?? d.output ?? '').toString();
  } catch {
    return '';
  }
}

// ── Account panel (shared; one linggen.dev account for every app) ──
const AccountPanel: React.FC = () => {
  const [acct, setAcct] = useState<any>(null);
  const load = useCallback(() => {
    fetch('/api/account')
      .then((r) => (r.ok ? r.json() : null))
      .then(setAcct)
      .catch(() => setAcct(null));
  }, []);
  useEffect(() => { load(); }, [load]);

  const signIn = () => {
    const host = window.location.host;
    window.open(`${window.location.protocol}//${host}/api/account/login?host=${encodeURIComponent(host)}`, '_blank', 'width=500,height=640');
    const t = setInterval(() => {
      fetch('/api/account').then((r) => r.ok ? r.json() : null).then((d) => {
        if (d?.signed_in) { clearInterval(t); setAcct(d); }
      }).catch(() => {});
    }, 1500);
    setTimeout(() => clearInterval(t), 120000);
  };
  const signOut = async () => { await fetch('/api/account/logout', { method: 'POST' }).catch(() => {}); load(); };

  return (
    <div className="lg-set-panel">
      <h2>Account</h2>
      <p className="lg-set-desc">AI features run through your linggen.dev account (free trial included). One account covers every app.</p>
      {acct === null ? (
        <p className="lg-set-muted">Checking sign-in…</p>
      ) : acct.signed_in ? (
        <div className="lg-set-card">
          <div className="lg-set-row">
            <div>
              <div className="lg-set-strong">{acct.user_name || 'Signed in'}</div>
              <div className="lg-set-muted">{acct.source === 'remote' ? 'Linked via linggen.dev' : 'linggen.dev'}</div>
            </div>
            <button className="lg-set-btn" onClick={signOut}>Sign out</button>
          </div>
        </div>
      ) : (
        <div className="lg-set-card">
          <div className="lg-set-row">
            <div className="lg-set-muted">Not signed in. Deterministic features work offline; AI needs an account.</div>
            <button className="lg-set-btn lg-set-btn-primary" onClick={signIn}>Sign in</button>
          </div>
        </div>
      )}
    </div>
  );
};

// ── Yinyue / desktop pet panel (shared; the pet is app-wide) ──
const YinyuePanel: React.FC = () => {
  const [shown, setShown] = useState<boolean | null>(null);
  const [onTop, setOnTop] = useState<boolean | null>(null);

  useEffect(() => {
    bash('[ -f ~/.linggen/pet-disabled ] && echo off || echo on; [ -f ~/.linggen/pet-always-on ] && echo on || echo off')
      .then((out) => {
        const [s, t] = out.trim().split('\n');
        setShown(s !== 'off');
        setOnTop(t === 'on');
      });
  }, []);

  const toggleShown = async () => {
    const next = !shown; setShown(next);
    await bash(next ? 'rm -f ~/.linggen/pet-disabled' : 'touch ~/.linggen/pet-disabled');
  };
  const toggleOnTop = async () => {
    const next = !onTop; setOnTop(next);
    await bash(next ? 'touch ~/.linggen/pet-always-on' : 'rm -f ~/.linggen/pet-always-on');
  };

  return (
    <div className="lg-set-panel">
      <h2>Yinyue</h2>
      <p className="lg-set-desc">Your companion on screen. Drag her body to move her anywhere.</p>
      <div className="lg-set-card">
        <label className="lg-set-row lg-set-toggle">
          <span>Show Yinyue</span>
          <input type="checkbox" checked={!!shown} onChange={toggleShown} disabled={shown === null} />
        </label>
      </div>
      <div className="lg-set-card">
        <label className="lg-set-row lg-set-toggle">
          <span>Always on top</span>
          <input type="checkbox" checked={!!onTop} onChange={toggleOnTop} disabled={onTop === null} />
        </label>
      </div>
    </div>
  );
};

export const LauncherSettings: React.FC<{ onClose: () => void }> = ({ onClose }) => {
  const [apps, setApps] = useState<AppSkill[]>([]);
  const [active, setActive] = useState<string>('account');

  useEffect(() => {
    fetch('/api/skills')
      .then((r) => (r.ok ? r.json() : []))
      .then((data) => {
        const list: any[] = Array.isArray(data) ? data : data?.skills ?? [];
        setApps(list.filter((s) => s.app && s.app.launcher === 'web' && HAS_SETTINGS.has(s.name)));
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') onClose(); };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  const sections: Section[] = [
    { id: 'account', label: 'Account', kind: 'account' },
    { id: 'model', label: 'Model', kind: 'model' },
    { id: 'yinyue', label: 'Yinyue', kind: 'yinyue' },
    ...apps.map((a) => ({ id: `app-${a.name}`, label: labelFor(a.name), kind: 'app' as const, skill: a })),
  ];
  const current = sections.find((s) => s.id === active) ?? sections[0];

  return (
    <div className="lg-set-overlay">
      <style>{`
        .lg-set-overlay { position: fixed; inset: 0; z-index: 60; display: flex; align-items: center; justify-content: center; background: rgba(0,0,0,0.45); }
        .lg-set-modal { width: min(920px, 92vw); height: min(660px, 88vh); background: #fff; color: #0f172a; border-radius: 14px; box-shadow: 0 24px 64px rgba(0,0,0,0.35); display: flex; flex-direction: column; overflow: hidden; }
        html.dark .lg-set-modal { background: #131313; color: #e2e8f0; }
        .lg-set-head { display:flex; align-items:center; justify-content:space-between; padding: 12px 16px; border-bottom: 1px solid rgba(120,120,120,0.18); flex-shrink: 0; }
        .lg-set-title { font-weight: 700; font-size: 14px; }
        .lg-set-x { background: none; border: none; cursor: pointer; color: #94a3b8; padding: 4px; border-radius: 6px; display: flex; }
        .lg-set-x:hover { background: rgba(120,120,120,0.14); }
        .lg-set-body { flex: 1; display: flex; min-height: 0; }
        .lg-set-nav { width: 180px; flex-shrink: 0; border-right: 1px solid rgba(120,120,120,0.18); padding: 8px; overflow-y: auto; }
        .lg-set-nav-item { display:block; width: 100%; text-align: left; padding: 7px 10px; border-radius: 8px; font-size: 13px; background: none; border: none; cursor: pointer; color: inherit; }
        .lg-set-nav-item:hover { background: rgba(120,120,120,0.1); }
        .lg-set-nav-item.active { background: rgba(120,120,120,0.18); font-weight: 600; }
        .lg-set-nav-div { height: 1px; margin: 8px 6px; background: rgba(120,120,120,0.18); }
        .lg-set-content { flex: 1; min-width: 0; overflow: auto; }
        .lg-set-frame { width: 100%; height: 100%; border: none; display: block; }
        .lg-set-panel { padding: 20px 24px; }
        .lg-set-panel h2 { font-size: 16px; font-weight: 700; margin: 0 0 4px; }
        .lg-set-desc { font-size: 12.5px; color: #64748b; margin: 0 0 14px; line-height: 1.5; }
        .lg-set-muted { font-size: 12.5px; color: #64748b; }
        .lg-set-strong { font-weight: 600; font-size: 13px; }
        .lg-set-card { border: 1px solid rgba(120,120,120,0.2); border-radius: 10px; padding: 12px 14px; margin-bottom: 10px; }
        .lg-set-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
        .lg-set-toggle { cursor: pointer; }
        .lg-set-btn { font-size: 12.5px; padding: 6px 12px; border-radius: 8px; border: 1px solid rgba(120,120,120,0.32); background: none; cursor: pointer; color: inherit; white-space: nowrap; }
        .lg-set-btn:hover { background: rgba(120,120,120,0.1); }
        .lg-set-btn-primary { background: #10b981; color: #fff; border-color: #10b981; }
        .lg-set-btn-primary:hover { background: #059669; }
      `}</style>
      <div className="lg-set-modal">
        <div className="lg-set-head">
          <span className="lg-set-title">Linggen Settings</span>
          <button className="lg-set-x" onClick={onClose} title="Close (Esc)"><X size={16} /></button>
        </div>
        <div className="lg-set-body">
          <nav className="lg-set-nav">
            {sections.map((s, i) => (
              <React.Fragment key={s.id}>
                {i === 3 && <div className="lg-set-nav-div" />}
                <button
                  className={`lg-set-nav-item${s.id === active ? ' active' : ''}`}
                  onClick={() => setActive(s.id)}
                >
                  {s.label}
                </button>
              </React.Fragment>
            ))}
          </nav>
          <div className="lg-set-content">
            {current?.kind === 'account' && <AccountPanel />}
            {current?.kind === 'yinyue' && <YinyuePanel />}
            {current?.kind === 'model' && (
              <iframe className="lg-set-frame" src="/settings/models" title="Model" />
            )}
            {current?.kind === 'app' && current.skill && (
              <iframe
                className="lg-set-frame"
                src={`/apps/${current.skill.name}/scripts/settings.html?embed=1`}
                title={labelFor(current.skill.name)}
              />
            )}
          </div>
        </div>
      </div>
    </div>
  );
};
