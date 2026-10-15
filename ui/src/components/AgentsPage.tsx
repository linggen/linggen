/**
 * Settings → Agents: the two agents and their models on top, the agent spec
 * files below — one place for everything about who the agents are.
 */
import React from 'react';
import type { AppConfig } from '../types';
import { AgentsSection } from './AgentsSection';
import { AgentsTab } from './AgentsTab';
import { Section } from './settings/kit';

export const AgentsPage: React.FC<{
  config: AppConfig;
  onChange: (config: AppConfig) => void;
  projectRoot: string;
}> = ({ config, onChange, projectRoot }) => (
  <div className="space-y-6">
    <AgentsSection config={config} onChange={onChange} />
    <Section title="Agent files" desc="Each agent's spec — personality, tools, prompt. Saved on its own.">
      <div className="h-[560px] rounded-lg border border-slate-200 dark:border-white/10 overflow-hidden">
        <AgentsTab projectRoot={projectRoot} />
      </div>
    </Section>
  </div>
);
