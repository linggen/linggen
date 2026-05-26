/** Mission CRUD endpoints. */
import type { CronMission } from '../types';
import { apiGet, apiPost, apiPut, apiDelete } from './api';

export interface MissionCreateArgs {
  name?: string;
  description?: string;
  schedule: string;
  prompt?: string;
  model?: string;
  cwd?: string;
  entry?: string;
  permission_mode?: string;
  permission_paths?: string[];
  permission_warning?: string;
  allowed_tools?: string[];
}

export async function fetchMissions(): Promise<CronMission[]> {
  try {
    const data = await apiGet<{ missions?: CronMission[] }>('/api/missions');
    return Array.isArray(data.missions) ? data.missions : [];
  } catch {
    return [];
  }
}

export function createMission(args: MissionCreateArgs): Promise<CronMission | null> {
  return apiPost<CronMission | null>('/api/missions', {
    name: args.name || null,
    description: args.description || '',
    schedule: args.schedule,
    prompt: args.prompt || null,
    model: args.model || null,
    cwd: args.cwd || null,
    entry: args.entry || null,
    permission_mode: args.permission_mode || 'admin',
    permission_paths: args.permission_paths || [],
    permission_warning: args.permission_warning || null,
    allowed_tools: args.allowed_tools || [],
  });
}

export function updateMission(id: string, updates: Record<string, any>): Promise<CronMission | null> {
  return apiPut<CronMission | null>(`/api/missions/${encodeURIComponent(id)}`, updates);
}

export function deleteMission(id: string): Promise<void> {
  return apiDelete<void>(`/api/missions/${encodeURIComponent(id)}`);
}

/** Raw `mission.md` content. The UI edits the file directly via CodeMirror;
 *  there is no structured form. The mission's frontmatter is the source of
 *  truth for all metadata. */
export function getMissionFile(id: string): Promise<{ id: string; content: string }> {
  return apiGet<{ id: string; content: string }>(
    `/api/mission-file?id=${encodeURIComponent(id)}`,
  );
}

/** Upsert a mission by writing its raw `mission.md`. The backend validates
 *  the frontmatter and reloads the schedule cache. */
export function saveMissionFile(id: string, content: string): Promise<CronMission> {
  return apiPost<CronMission>('/api/mission-file', { id, content });
}
