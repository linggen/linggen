/**
 * Settings save themselves. A change to the config file is written once the
 * person stops changing it for a moment — a toggle at once, typing when it
 * pauses. The engine validates each write; a refused one is shown and kept
 * dirty, and the next edit tries again. (API keys are not here: the Models
 * page sends a key to its own route.)
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type { AppConfig } from '../../types';
import { appConfig } from '../../lib/endpoints';
import { apiErrorMessage } from '../../lib/api';
import { rememberLanguage } from '../../lib/i18n';
import { useServerStore } from '../../stores/serverStore';
import { settleWrites } from '../../lib/settleWrites.mts';

/** Quiet time after the last edit before it is written. */
const SETTLE_MS = 700;

export type SaveState = 'idle' | 'saving' | 'saved' | 'error';

export function useAutoSaveConfig(enabled = true) {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [saved, setSaved] = useState<AppConfig | null>(null);
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

  // One pending write; leaving Settings writes it at once (lib/settleWrites).
  const writes = useState(() => settleWrites(SETTLE_MS))[0];
  useEffect(() => () => writes.flush(), [writes]);

  // What the page holds, for `reload` to read when it is called.
  const held = useRef({ config, saved });
  useEffect(() => { held.current = { config, saved }; });

  /** A route of the engine changed the config (a family ticked, a default
   *  made): read it again, so no later save posts the old copy back — a save
   *  of a copy older than a tick would remove the ticked row. An edit still
   *  waiting is written first. */
  const reload = useCallback(async () => {
    const { config: edited, saved: written } = held.current;
    writes.cancel();
    if (edited && written && JSON.stringify(edited) !== JSON.stringify(written)) await appConfig.save(edited);
    const data = await appConfig.get();
    setConfig(data);
    setSaved(data);
    useServerStore.setState({ defaultModels: data.routing?.default_models ?? [] });
  }, [writes]);

  useEffect(() => {
    if (!config || !saved || !configDirty) {
      writes.cancel();
      return;
    }
    writes.later(async () => {
      setState('saving');
      setError(null);
      try {
        await appConfig.save(config);
        setSaved(config);
        // The chat's model pickers read the defaults from here.
        useServerStore.setState({ defaultModels: config.routing?.default_models ?? [] });
        // A new language is the whole page's: reload it into the new words.
        if (config.language !== saved.language) {
          rememberLanguage(config.language ?? null);
          (window.top ?? window).location.reload();
          return;
        }
        setState('saved');
      } catch (e) {
        setError(apiErrorMessage(e) || `Save failed: ${e}`);
        setState('error');
      }
    });
  }, [config, saved, configDirty, writes]);

  // "Saved" fades after a moment; an error stays until the next save.
  useEffect(() => {
    if (state !== 'saved') return;
    const timer = setTimeout(() => setState('idle'), 1800);
    return () => clearTimeout(timer);
  }, [state]);

  return { config, setConfig, reload, state, error };
}
