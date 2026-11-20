import { useEffect, useRef } from 'react';
import { X } from 'lucide-react';
import { SettingsPanel } from '../pages/Settings/SettingsPanel';
import { watchTopDialogEscape } from '../lib/dialogEscape.mts';

export function LauncherSettings({ onClose, request }: { onClose: () => void; request: { section?: string } }) {
  const dialog = useRef<HTMLDivElement>(null);
  useEffect(() => { dialog.current?.focus(); }, []);
  useEffect(() => watchTopDialogEscape(dialog.current!, onClose), [onClose]);

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/45">
      <div ref={dialog} tabIndex={-1} role="dialog" aria-modal="true" aria-label="Linggen Settings" className="w-[min(920px,92vw)] h-[min(660px,88vh)] overflow-hidden rounded-[14px] bg-white dark:bg-[#131313] shadow-2xl">
        <SettingsPanel
          initial={request}
          canOpenRun={false}
          heading={<span className="text-sm font-bold">Linggen Settings</span>}
          trailing={<button className="flex rounded-md p-1 text-slate-400 hover:bg-slate-500/15" onClick={onClose} title="Close (Esc)"><X size={16} /></button>}
        />
      </div>
    </div>
  );
}
