import { useCallback } from 'react';
import { useNavigate } from 'react-router-dom';
import type { SettingsTab } from '../lib/settingsSections.mts';

/** Route-state shape consumed by SettingsHome to pre-select a tab. */
export interface SettingsLocationState {
  tab?: SettingsTab;
}

/** Navigate to /settings, optionally selecting a specific tab. The tab is
 *  passed via React Router location state — no global side-channel. */
export function useOpenSettings() {
  const navigate = useNavigate();
  return useCallback((tab?: SettingsTab) => {
    const state: SettingsLocationState = { tab };
    navigate('/settings', { state });
  }, [navigate]);
}
