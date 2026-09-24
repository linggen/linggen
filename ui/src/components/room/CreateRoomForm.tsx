import React, { useState } from 'react';
import { BudgetInput } from './BudgetInput';
import type { NewRoom } from './roomApi';
import { btnPrimary, inputCls, sectionCls } from './roomStyles';

const fieldLabel = 'block text-[10px] font-bold text-slate-400 mb-1';

export const CreateRoomForm: React.FC<{
  saving: boolean;
  onCreate: (room: NewRoom) => void;
  onCancel: () => void;
}> = ({ saving, onCreate, onCancel }) => {
  const [name, setName] = useState('My Room');
  const [type, setType] = useState('private');
  const [maxConsumers, setMaxConsumers] = useState(4);
  const [budget, setBudget] = useState<number | null>(500000);

  return (
    <div className={sectionCls}>
      <div className="space-y-3">
        <div>
          <label className={fieldLabel}>Name</label>
          <input value={name} onChange={e => setName(e.target.value)} className={inputCls} />
        </div>
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className={fieldLabel}>Type</label>
            <select value={type} onChange={e => setType(e.target.value)} className={inputCls}>
              <option value="private">Private</option>
              <option value="public">Public</option>
            </select>
          </div>
          <div>
            <label className={fieldLabel}>Max Users</label>
            <select value={maxConsumers} onChange={e => setMaxConsumers(parseInt(e.target.value))} className={inputCls}>
              {[1, 2, 3, 4].map(n => <option key={n} value={n}>{n}</option>)}
            </select>
          </div>
        </div>
        <div>
          <label className={fieldLabel}>Daily Budget</label>
          <BudgetInput value={budget} onChange={setBudget} />
        </div>
      </div>
      <div className="flex gap-2 pt-1">
        <button
          onClick={() => onCreate({ name: name || 'My Room', room_type: type, max_consumers: maxConsumers, token_budget_daily: budget })}
          disabled={saving}
          className={btnPrimary}
        >
          {saving ? 'Creating...' : 'Create'}
        </button>
        <button onClick={onCancel} className="text-xs text-slate-500 hover:text-slate-700 dark:hover:text-slate-300">
          Cancel
        </button>
      </div>
    </div>
  );
};
