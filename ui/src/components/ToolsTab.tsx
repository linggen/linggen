import React, { useCallback, useEffect, useState } from 'react';
import { Eye, EyeOff, ExternalLink } from 'lucide-react';

const inputCls = 'w-full bg-slate-100 dark:bg-white/5 border border-slate-200 dark:border-white/10 rounded-lg px-3 py-2 text-xs outline-none focus:ring-1 focus:ring-blue-500/50';
const sectionCls = 'bg-white dark:bg-[#141414] rounded-xl border border-slate-200 dark:border-white/5 shadow-sm p-5';

type CredentialsMap = Record<string, { api_key?: string | null }>;

interface ToolDef {
  name: string;
  description: string;
  credentialKey?: string; // key in credentials.json, e.g. "tavily"
}

const TOOLS: ToolDef[] = [
  { name: 'Read', description: 'Read files' },
  { name: 'Write', description: 'Write files' },
  { name: 'Edit', description: 'Edit files' },
  { name: 'Bash', description: 'Run shell commands' },
  { name: 'Glob', description: 'Find files by pattern' },
  { name: 'Grep', description: 'Search file contents' },
  { name: 'WebSearch', description: 'Search the web (Tavily)', credentialKey: 'tavily' },
  { name: 'WebFetch', description: 'Fetch web page content' },
  { name: 'Skill', description: 'Run skills' },
  { name: 'AskUser', description: 'Ask user questions' },
  { name: 'Task', description: 'Delegate to subagent' },
];

export const ToolsTab: React.FC = () => {
  const [credentials, setCredentials] = useState<CredentialsMap>({});
  const [localKeys, setLocalKeys] = useState<Record<string, string>>({});
  const [revealKeys, setRevealKeys] = useState<Record<string, boolean>>({});
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);

  const fetchCredentials = useCallback(async () => {
    try {
      const resp = await fetch('/api/credentials');
      if (resp.ok) {
        const data: CredentialsMap = await resp.json();
        setCredentials(data);
        const keys: Record<string, string> = {};
        for (const tool of TOOLS) {
          if (tool.credentialKey) {
            keys[tool.credentialKey] = data[tool.credentialKey]?.api_key || '';
          }
        }
        setLocalKeys(keys);
      }
    } catch { /* ignore */ }
  }, []);

  useEffect(() => { fetchCredentials(); }, [fetchCredentials]);

  const hasKey = (credKey: string) => !!(credentials[credKey]?.api_key);

  const updateLocalKey = (credKey: string, value: string) => {
    setLocalKeys((prev) => ({ ...prev, [credKey]: value }));
    setDirty(true);
  };

  const saveCredentials = async () => {
    if (saving) return;
    setSaving(true);
    const body: CredentialsMap = {};
    for (const tool of TOOLS) {
      if (!tool.credentialKey) continue;
      const val = localKeys[tool.credentialKey];
      if (val !== undefined && val !== '***') {
        body[tool.credentialKey] = { api_key: val || null };
      }
    }
    try {
      await fetch('/api/credentials', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      setDirty(false);
      fetchCredentials();
    } catch { /* ignore */ }
    setSaving(false);
  };

  return (
    <div className="space-y-6">
      <section className={sectionCls}>
        <h2 className="text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-1">
          Built-in Tools
        </h2>
        <p className="text-[11px] text-slate-400 mb-4">
          Tools available to agents during execution. Some tools require API keys to function.
        </p>

        <div className="space-y-1">
          {TOOLS.map((tool) => (
            <div key={tool.name} className="flex items-center gap-3 px-3 py-2.5 bg-slate-50 dark:bg-white/[0.02] rounded-lg border border-slate-100 dark:border-white/5">
              <span className="text-xs font-mono font-semibold text-slate-700 dark:text-slate-200 w-40 shrink-0">
                {tool.name}
              </span>
              <span className="text-[12px] text-slate-500 dark:text-slate-400 flex-1">
                {tool.description}
              </span>
              {tool.credentialKey && (
                <div className="flex items-center gap-2 shrink-0 w-72">
                  <label className="text-[11px] font-bold uppercase tracking-wider text-slate-500 dark:text-slate-400 shrink-0">
                    API Key
                    {hasKey(tool.credentialKey) && <span className="ml-1 text-green-500 text-[8px]">(set)</span>}
                  </label>
                  <div className="relative flex-1">
                    <input
                      className={inputCls + ' pr-8 !py-1.5'}
                      type={revealKeys[tool.credentialKey] ? 'text' : 'password'}
                      value={localKeys[tool.credentialKey] ?? ''}
                      onChange={(e) => updateLocalKey(tool.credentialKey!, e.target.value)}
                      placeholder={hasKey(tool.credentialKey) ? '(stored in ~/.linggen/credentials.json)' : 'Enter API key'}
                    />
                    <button
                      type="button"
                      className="absolute right-2 top-1/2 -translate-y-1/2 text-slate-400 hover:text-slate-600"
                      onClick={() => setRevealKeys((prev) => ({ ...prev, [tool.credentialKey!]: !prev[tool.credentialKey!] }))}
                      tabIndex={-1}
                    >
                      {revealKeys[tool.credentialKey] ? <EyeOff size={12} /> : <Eye size={12} />}
                    </button>
                  </div>
                </div>
              )}
            </div>
          ))}
        </div>

        {dirty && (
          <div className="mt-4 flex justify-end">
            <button
              onClick={saveCredentials}
              disabled={saving}
              className="px-3 py-1.5 text-xs font-medium bg-blue-600 text-white rounded-lg hover:bg-blue-700 transition-colors"
            >
              {saving ? 'Saving...' : 'Save API Keys'}
            </button>
          </div>
        )}

        <div className="mt-3 flex items-center gap-4">
          <p className="text-[11px] text-slate-400">
            API keys are stored in <code className="text-[11px]">~/.linggen/credentials.json</code>, not in the config file.
          </p>
          <a
            href="https://tavily.com"
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1 text-[11px] text-blue-500 hover:text-blue-600 transition-colors shrink-0"
          >
            Get a free Tavily API key <ExternalLink size={10} />
          </a>
        </div>
      </section>
    </div>
  );
};
