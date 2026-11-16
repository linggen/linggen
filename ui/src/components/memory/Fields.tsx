import React, { useId } from 'react';
import { inputCls, labelCls } from './memoryStyle';

/** A labelled drop-down. `none` adds an empty choice with that label. */
export const PickField: React.FC<{
  label: string; value: string; options: readonly string[]; none?: string;
  show?: (v: string) => string; onChange: (v: string) => void;
}> = ({ label, value, options, none, show, onChange }) => {
  const id = useId();
  return (
    <div>
      <label htmlFor={id} className={labelCls}>{label}</label>
      <select id={id} className={`${inputCls} min-h-11 md:min-h-0 mt-1`} value={value} onChange={(e) => onChange(e.target.value)}>
        {none !== undefined && <option value="">{none}</option>}
        {options.map((o) => <option key={o} value={o}>{show ? show(o) : o}</option>)}
      </select>
    </div>
  );
};

export const TextField: React.FC<{
  label: string; value: string; hint?: string; placeholder?: string; disabled?: boolean; onChange: (v: string) => void;
}> = ({ label, value, hint, placeholder, disabled, onChange }) => {
  const id = useId();
  return (
    <div>
      <label htmlFor={id} className={labelCls}>{label}</label>
      <input
        id={id}
        data-no-i18n
        className={`${inputCls} min-h-11 md:min-h-0 mt-1 disabled:opacity-50`}
        value={value}
        disabled={disabled}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
      />
      {hint && <p className="text-[11px] text-slate-400 mt-1">{hint}</p>}
    </div>
  );
};
