import React, { useState } from 'react';
import { inputCls } from './roomStyles';

/** Budget input — displays in K units with comma formatting, stores raw tokens. */
export const BudgetInput: React.FC<{
  value: number | null;
  onChange: (val: number | null) => void;
  className?: string;
}> = ({ value, onChange, className }) => {
  const kText = value != null ? String(Math.round(value / 1000)) : '';
  const [text, setText] = useState(kText);
  // Follow a new value from outside (adjusting state during render, not in
  // an effect, so the input never shows the stale number for a frame).
  const [lastValue, setLastValue] = useState(value);
  if (value !== lastValue) {
    setLastValue(value);
    setText(kText);
  }

  const display = text === '' ? '' : Number(text.replace(/,/g, '') || 0).toLocaleString();

  return (
    <div className="flex items-center gap-1.5">
      <input
        type="text"
        placeholder="500"
        value={display}
        onChange={e => setText(e.target.value.replace(/[^0-9]/g, ''))}
        onBlur={() => {
          const raw = parseInt(text.replace(/,/g, '') || '0');
          onChange(raw > 0 ? raw * 1000 : null);
        }}
        onKeyDown={e => {
          if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return;
          e.preventDefault();
          const cur = parseInt(text.replace(/,/g, '') || '0');
          const next = e.key === 'ArrowUp' ? cur + 1 : Math.max(0, cur - 1);
          setText(String(next));
          onChange(next > 0 ? next * 1000 : null);
        }}
        className={className || inputCls}
      />
      <span className="text-[10px] text-slate-500 font-medium shrink-0">K</span>
    </div>
  );
};
