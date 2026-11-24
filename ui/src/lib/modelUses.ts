export type UseOption = { id: string; label: string; state: 'ready' | 'download' | 'no_key' | 'signed_out' | 'unavailable'; disk_gb?: number };
export type UseRow = { choice: string | null; effective: string | null; options: UseOption[] };
export type Uses = Record<'chat' | 'effort' | 'live' | 'speak' | 'voice' | 'draw' | 'hear', UseRow> & {
  chain: string[];
  apps: { id: string; label: string; choice: string | null }[];
};
export type UseEdit = { use: 'chat' | 'effort' | 'voice'; choice: string }
  | { use: 'live' | 'speak' | 'draw'; choice: string | null }
  | { use: 'app'; id: string; choice: string | null };
