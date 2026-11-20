import { useCallback } from 'react';
import { useLocation, useMatch, useNavigate } from 'react-router-dom';
import type { SettingsTarget } from '../lib/settingsSections.mts';

/** Route-state shape consumed by SettingsHome to pre-select a tab. */
export interface SettingsLocationState {
  tab?: SettingsTarget;
  returnToLauncher?: boolean;
}

/** Navigate to /settings, optionally selecting a specific tab. The tab is
 *  passed via React Router location state — no global side-channel. */
export function useOpenSettings({ launcher = false } = {}) {
  const navigate = useNavigate();
  const onSettings = !!useMatch('/settings');
  const location = useLocation();
  const returnToLauncher = launcher && (!onSettings || !!(location.state as SettingsLocationState | null)?.returnToLauncher);
  return useCallback((tab?: SettingsTarget) => {
    if (launcher && onSettings && !tab) return;
    const state: SettingsLocationState = { tab, returnToLauncher };
    navigate('/settings', { state, replace: launcher && onSettings });
  }, [navigate, launcher, onSettings, returnToLauncher]);
}
