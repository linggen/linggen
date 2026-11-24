export type NotReady = 'not_signed_in' | 'sign_in_expired' | 'provider_sign_in' | 'no_key' | 'usage_limit' | 'rate_limited' | 'quota_used_up' | 'provider_failing';
export type UseOption = { id: string; label: string; state: 'ready' | 'download' | 'no_key' | 'signed_out' | 'unavailable'; disk_gb?: number; reason?: NotReady };
export type UseRow = { choice: string | null; effective: string | null; options: UseOption[] };
export type Uses = Record<'chat' | 'effort' | 'live' | 'speak' | 'voice' | 'draw' | 'hear', UseRow> & {
  chain: string[];
  model_fallback: boolean;
  apps: { id: string; label: string; choice: string | null }[];
};
export type UseEdit = { use: 'chat' | 'effort' | 'voice'; choice: string }
  | { use: 'chat_fallbacks'; choices: string[] }
  | { use: 'model_fallback'; enabled: boolean }
  | { use: 'live' | 'speak' | 'draw'; choice: string | null }
  | { use: 'app'; id: string; choice: string | null };
