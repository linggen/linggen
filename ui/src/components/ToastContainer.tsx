import React from 'react';
import { useUiStore, type Toast } from '../stores/uiStore';
import { X, CheckCircle, AlertCircle, Info } from 'lucide-react';

const variantStyles: Record<Toast['variant'], { bg: string; icon: React.ReactNode }> = {
  success: {
    bg: 'bg-emerald-600 dark:bg-emerald-700',
    icon: <CheckCircle size={14} />,
  },
  error: {
    bg: 'bg-red-600 dark:bg-red-700',
    icon: <AlertCircle size={14} />,
  },
  info: {
    bg: 'bg-blue-600 dark:bg-blue-700',
    icon: <Info size={14} />,
  },
};

export const ToastContainer: React.FC = () => {
  const toasts = useUiStore((s) => s.toasts);
  const removeToast = useUiStore((s) => s.removeToast);

  if (toasts.length === 0) return null;

  return (
    <div className="fixed bottom-4 right-4 z-[9999] flex flex-col gap-2 pointer-events-none">
      {toasts.map((t) => {
        const style = variantStyles[t.variant];
        return (
          <div
            key={t.id}
            onClick={() => {
              t.onClick?.();
              removeToast(t.id);
            }}
            className={`pointer-events-auto flex items-center gap-2 px-3 py-2 rounded-lg shadow-lg text-white text-[13px] font-medium max-w-xs animate-slide-in ${style.bg} ${t.onClick ? 'cursor-pointer hover:brightness-110' : ''}`}
          >
            {style.icon}
            <span className="flex-1">{t.message}</span>
            <button
              onClick={(e) => { e.stopPropagation(); removeToast(t.id); }}
              className="shrink-0 p-0.5 rounded hover:bg-white/20 transition-colors cursor-pointer"
            >
              <X size={12} />
            </button>
          </div>
        );
      })}
    </div>
  );
};
