/**
 * Settings save themselves. A change to the config file, or to the API keys
 * (`credentials.json`), is written once the person stops changing it for a
 * moment — a toggle at once, typing when it pauses. The engine validates each
 * write; a refused one is shown and kept dirty, and the next edit tries again.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { AppConfig } from '../../types';
import { appConfig } from '../../lib/endpoints';
import { apiErrorMessage } from '../../lib/api';
import { rememberLanguage } from '../../lib/i18n';
import { useServerStore } from '../../stores/serverStore';

/** Quiet time after the last edit before it is written. */
const SETTLE_MS = 700;

export type SaveState = 'idle' | 'saving' | 'saved' | 'error';

export function useAutoSaveConfig(enabled = true) {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [saved, setSaved] = useState<AppConfig | null>(null);
  const [credsDirty, setCredsDirty] = useState(false);
  const saveCredsRef = useRef<(() => Promise<void>) | null>(null);
  const [state, setState] = useState<SaveState>('idle');
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const data = await appConfig.get();
      setConfig(data);
      setSaved(data);
      setError(null);
    } catch (e) {
      setError(`Failed to load config: ${e}`);
    }
  }, []);

  useEffect(() => { if (enabled) load(); }, [enabled, load]);

  const configDirty = config !== null && saved !== null && JSON.stringify(config) !== JSON.stringify(saved);

  useEffect(() => {
    if (!config || !saved || (!configDirty && !credsDirty)) return;
    const timer = setTimeout(async () => {
      setState('saving');
      setError(null);
      try {
        if (configDirty) {
          await appConfig.save(config);
          setSaved(config);
          // The chat's model pickers read the defaults from here.
          useServerStore.setState({
            defaultModels: config.routing?.default_models ?? [],
            claudeDefaultModel: config.routing?.claude_default_model ?? '',
          });
          // A new language is the whole page's: reload it into the new words.
          if (config.language !== saved.language) {
            rememberLanguage(config.language ?? null);
            (window.top ?? window).location.reload();
            return;
          }
        }
        if (credsDirty && saveCredsRef.current) await saveCredsRef.current();
        setState('saved');
      } catch (e) {
        setError(apiErrorMessage(e) || `Save failed: ${e}`);
        setState('error');
      }
    }, SETTLE_MS);
    return () => clearTimeout(timer);
  }, [config, saved, configDirty, credsDirty]);

  // "Saved" fades after a moment; an error stays until the next save.
  useEffect(() => {
    if (state !== 'saved') return;
    const timer = setTimeout(() => setState('idle'), 1800);
    return () => clearTimeout(timer);
  }, [state]);

  return { config, setConfig, setCredsDirty, saveCredsRef, state, error };
}
