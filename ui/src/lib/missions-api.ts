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

export function updateMission(id: string, updates: Partial<CronMission>): Promise<CronMission | null> {
  return apiPut<CronMission | null>(`/api/missions/${encodeURIComponent(id)}`, updates);
}

/** Run a mission now. Axum's Json extractor needs a real JSON body — a
 *  bodyless POST 415s silently. */
export function triggerMission(id: string): Promise<void> {
  return apiPost<void>(`/api/missions/${encodeURIComponent(id)}/trigger`, {});
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
