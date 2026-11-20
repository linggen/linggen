import { useEffect } from 'react';
import { X } from 'lucide-react';
import { SettingsPanel } from '../pages/Settings/SettingsPanel';

export function LauncherSettings({ onClose, initial }: { onClose: () => void; initial?: string }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !e.defaultPrevented && !document.querySelector('[data-settings-dialog]')) onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/45">
      <div role="dialog" aria-modal="true" aria-label="Linggen Settings" className="w-[min(920px,92vw)] h-[min(660px,88vh)] overflow-hidden rounded-[14px] bg-white dark:bg-[#131313] shadow-2xl">
        <SettingsPanel
          initial={initial}
          fallback="account"
          heading={<span className="text-sm font-bold">Linggen Settings</span>}
          trailing={<button className="flex rounded-md p-1 text-slate-400 hover:bg-slate-500/15" onClick={onClose} title="Close (Esc)"><X size={16} /></button>}
        />
      </div>
    </div>
  );
}
