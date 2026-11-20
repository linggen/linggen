import { inShell } from '../lib/appHost';
import { COMPANION_ID } from '../lib/agentsMet.mts';
import type { SettingsRequirements } from '../lib/settingsSections.mts';
import { useUserStore } from '../stores/userStore';
import { useAgentPresent } from './useAgentPresent';

export function useSettingsRequirements(): SettingsRequirements {
  const rooms = useUserStore((s) => s.roomsOn);
  const companion = useAgentPresent(COMPANION_ID);
  return { rooms: rooms === true, companion, shell: inShell() };
}
