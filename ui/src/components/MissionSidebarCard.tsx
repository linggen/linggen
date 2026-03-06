import React, { useEffect, useRef, useState } from 'react';
import { ChevronRight } from 'lucide-react';
import type { CronMission } from '../types';

interface MissionSidebarCardProps {
  projectRoot: string | null;
  onOpenMission: () => void;
}

export const MissionSidebarCard: React.FC<MissionSidebarCardProps> = ({
  projectRoot,
  onOpenMission,
}) => {
  const [missions, setMissions] = useState<CronMission[]>([]);
  const endpointAvailable = useRef<boolean | null>(null);

  useEffect(() => {
    endpointAvailable.current = null;
  }, [projectRoot]);

  useEffect(() => {
    if (!projectRoot) return;
    if (endpointAvailable.current === false) return;
    const fetchMissions = async () => {
      try {
        const url = new URL('/api/missions', window.location.origin);
        url.searchParams.append('project_root', projectRoot);
        const resp = await fetch(url.toString());
        if (resp.status === 404) {
          endpointAvailable.current = false;
          return;
        }
        endpointAvailable.current = true;
        if (!resp.ok) return;
        const data = await resp.json();
        if (Array.isArray(data?.missions)) {
          setMissions(data.missions);
        }
      } catch {
        // silently ignore
      }
    };
    fetchMissions();
  }, [projectRoot]);

  const enabledMissions = missions.filter(m => m.enabled);

  return (
    <div className="px-3 py-2 text-xs space-y-2">
      {enabledMissions.length > 0 ? (
        <div className="space-y-1">
          {enabledMissions.slice(0, 3).map(m => (
            <div key={m.id} className="flex items-center gap-1.5">
              <span className="inline-block w-1.5 h-1.5 rounded-full bg-green-500 shrink-0" />
              <span className="text-[10px] font-mono text-blue-600 dark:text-blue-400">{m.schedule}</span>
              <span className="text-[10px] text-slate-500 truncate">{m.agent_id}</span>
            </div>
          ))}
          {enabledMissions.length > 3 && (
            <span className="text-[10px] text-slate-400">+{enabledMissions.length - 3} more</span>
          )}
        </div>
      ) : (
        <p className="text-slate-400 dark:text-slate-500 text-[10px] italic">No active missions</p>
      )}
      <button
        onClick={onOpenMission}
        className="w-full flex items-center justify-center gap-1 px-2 py-1 text-[10px] font-semibold rounded-md text-slate-500 dark:text-slate-400 hover:text-blue-600 dark:hover:text-blue-400 hover:bg-slate-50 dark:hover:bg-white/5 transition-colors"
      >
        {enabledMissions.length > 0 ? 'Manage' : 'Create Mission'}
        <ChevronRight size={10} />
      </button>
    </div>
  );
};
