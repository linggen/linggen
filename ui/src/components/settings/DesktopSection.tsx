import { useEffect, useState } from 'react';
import { workspaceApi } from '../../lib/endpoints';

async function bash(command: string): Promise<string> {
  try {
    const d = await workspaceApi.bash('/tmp', command);
    return (d.stdout ?? d.output ?? '').toString();
  } catch {
    return '';
  }
}

// The native shell watches these files. Presence is on, except for pet-disabled.
function FlagSwitch({ file, label, inverted = false }: { file: string; label: string; inverted?: boolean }) {
  const path = `"$HOME/.linggen/${file}"`;
  const [on, setOn] = useState<boolean | null>(null);
  useEffect(() => {
    bash(`[ -f ${path} ] && echo present || echo absent`).then((out) => {
      if (out.trim()) setOn((out.trim() === 'present') !== inverted);
    });
  }, [path, inverted]);

  const toggle = async () => {
    const next = !on;
    setOn(next);
    await bash(`${next !== inverted ? 'touch' : 'rm -f'} ${path}`);
  };

  return (
    <label className="flex cursor-pointer items-center justify-between gap-3 rounded-xl border border-slate-200 dark:border-white/10 p-3 text-xs">
      <span>{label}</span>
      <input type="checkbox" checked={on === true} onChange={toggle} disabled={on === null} />
    </label>
  );
}

export function DesktopSection({ companion }: { companion: boolean }) {
  return (
    <section className="space-y-3">
      <h2 className="text-sm font-bold">Desktop</h2>
      <p className="text-xs text-slate-500">How Linggen runs on your Mac.</p>
      <FlagSwitch file="start-at-login" label="Start at login" />
      <p className="text-xs text-slate-500">Launch Linggen in the background when you sign in. It waits in the menu bar; closing the window keeps it running there. Quit it from the menu-bar icon.</p>
      {companion && <>
        <FlagSwitch file="pet-disabled" label="Show Yinyue" inverted />
        <FlagSwitch file="pet-always-on" label="Always on top" />
      </>}
    </section>
  );
}
