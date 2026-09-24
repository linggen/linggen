// Wire types generated from the Rust structs (scripts/gen-types.sh).
export type { UiEvent, UiEventOf, UiEventByKind, RunEvent, RunEventOf, ContentBlockEvent, PageState } from './types/uiEvents';
export type { QueuedChatItem } from './types/generated/QueuedChatItem';
export type { Plan } from './types/generated/Plan';
export type { PlanItem } from './types/generated/PlanItem';
export type { PlanStatus } from './types/generated/PlanStatus';
export type { AskUserOption } from './types/generated/AskUserOption';
export type { AskUserQuestion } from './types/generated/AskUserQuestion';
export type { AskUserAnswer } from './types/generated/AskUserAnswer';
export type { AgentRunRecord as AgentRunInfo } from './types/generated/AgentRunRecord';
import type { AskUserQuestion } from './types/generated/AskUserQuestion';


export interface SubagentToolStep {
  toolName: string;
  args: string;
  /** Client-side stamp for ordering/diagnostics. */
  timestampMs?: number;
  status: 'running' | 'done' | 'failed';
}

export interface SubagentTreeEntry {
  subagentId: string;
  agentName: string;
  task: string;
  status: 'running' | 'done' | 'failed';
  toolCount: number;
  contextTokens: number;
  currentActivity: string | null;
  toolSteps: SubagentToolStep[];
  /** Final text the subagent returned (status line, summary, etc.). Captured
   * from the subagent's terminal Message event so it can be revealed in the
   * expand-on-click view instead of leaking into the parent chat. */
  resultText?: string;
  /** Client-side stamp for ordering/diagnostics (set when subagent is spawned). */
  timestampMs?: number;
}

export interface MessageSegment {
  type: 'text' | 'tools';
  text?: string;         // for 'text' segments
  entries?: string[];    // for 'tools' segments
}

/** Structured content block — Claude Code-style message model. */
export interface ContentBlock {
  type: 'text' | 'tool_use' | 'tool_result' | 'thinking';
  /** Client-side stamp (ms) set when the block is first inserted. Used for
   * ordering diagnostics — the renderer uses array insertion order, not this. */
  timestampMs?: number;
  id?: string;           // unique block ID (for tool_use blocks)
  text?: string;         // for text/thinking blocks
  tool?: string;         // for tool_use: "Read", "Edit", "Bash"
  args?: string;         // for tool_use: compact arg summary
  summary?: string;      // for tool_result: one-line result summary
  status?: 'running' | 'done' | 'failed';  // for tool_use lifecycle
  isError?: boolean;     // for tool_result
  output?: string[];     // accumulated stdout/stderr lines (Bash)
  diffData?: {           // Edit/Write diff data
    diff_type: 'edit' | 'write';
    path: string;
    old_string?: string;
    new_string?: string;
    new_content?: string;
    start_line?: number;
    lines_written?: number;
  };
}

export interface ChatMessage {
  role: 'user' | 'agent';
  from?: string;
  to?: string;
  text: string;
  timestamp: string;
  timestampMs?: number;
  isGenerating?: boolean;
  isThinking?: boolean;
  /** Finalized from a frozen mid-stream bubble (terminal events lost while
   *  the session channel was closed). Marks it claimable by the persisted
   *  row that holds the same turn's full text — see mergeChatMessages. */
  wasFrozenStream?: boolean;
  activitySummary?: string;
  activityEntries?: string[];
  contextTokens?: number;
  messageCount?: number;
  toolCount?: number;
  durationMs?: number;
  images?: string[];
  imageCount?: number;
  subagentTree?: SubagentTreeEntry[];
  segments?: MessageSegment[];
  liveText?: string;
  /** Structured content blocks (new message model). */
  content?: ContentBlock[];
  /** True when the message represents an error (agent loop failure, etc.). */
  isError?: boolean;
  /** On a "Message failed to send" line: what was sent, so a tap can resend it. */
  resend?: { text: string; agentId: string; images?: string[]; persisted?: boolean };
}



export interface FileEntry {
  name: string;
  isDir: boolean;
  path: string;
}

/** A persisted transcript row's header. */
export interface PersistedMeta {
  from: string;
  to?: string;
  ts: number;
  [key: string]: unknown;
}

export interface SessionState {
  active_task: [PersistedMeta, string] | null;
  user_stories: [PersistedMeta, string] | null;
  tasks: [PersistedMeta, string][];
  messages: [PersistedMeta, string][];
  agent_status?: string;
  plan_status?: string;
}

export interface AgentTreeItem {
  type: 'file' | 'dir';
  agent?: string;
  status?: string;
  path?: string;
  last_modified?: number;
  children?: Record<string, AgentTreeItem>;
}

export interface AgentWorkInfo {
  path: string;
  file: string;
  folder: string;
  status: string;
  activeCount: number;
}

export interface SubagentInfo {
  id: string;
  status: string;
  path: string;
  file: string;
  folder: string;
  activeCount: number;
  paths: string[];
}

/** @deprecated Use SkillInfoFull instead */
export type SkillInfo = SkillInfoFull;

export interface AgentInfo {
  name: string;
  description: string;
  model?: string | null;
  /** Other names a message may address it by (`@银月`). */
  aliases?: string[];
}

export interface AgentFileInfo {
  agent_id: string;
  name: string;
  description: string;
  path: string;
}

export interface ModelInfo {
  id: string;
  provider: string;
  model: string;
  url: string;
  reasoning_effort?: string | null;
  provided_by?: string | null;
  /** Runtime reading from /api/models: credentials present right now.
   *  Absent on older payloads — treat undefined as usable. */
  auth_ok?: boolean;
}

/** A row of GET /api/models: what page_state sends plus the fields only the
 *  settings screens need (built-in flag, sign-in mode). */
export interface RuntimeModelInfo extends Partial<ModelInfo> {
  id: string;
  is_builtin?: boolean;
  auth_mode?: string | null;
}

export interface OllamaPsModel {
  name: string;
  model: string;
  size: number;
  size_vram: number;
  details: {
    parameter_size: string;
    quantization_level: string;
  };
}

export interface OllamaPsResponse {
  models: OllamaPsModel[];
}

export interface AppConfig {
  models: ModelConfigUI[];
  server: { port: number; host: string };
  agent: { max_iters: number; write_safety_mode: string; tool_permission_mode: string; prompt_loop_breaker?: string | null; compact_threshold?: number | null; memory_inject_min_score?: number; memory_recall_count?: number; ling_mem_url?: string; suggest_followups?: boolean };
  logging: { level?: string | null; directory?: string | null; retention_days?: number | null };
  agents: { id: string; spec_path: string; model?: string | null }[];
  routing?: { default_models?: string[]; default_policy?: string | null; auto_fallback?: boolean };
  pet?: { enabled?: boolean; pet?: string; show_text?: boolean; recall_count?: number; recall_min_score?: number; model?: string; voice?: string; muted?: boolean };
}

export type ModelHealthStatus = 'healthy' | 'quota_exhausted' | 'down' | 'unknown';

export interface ModelHealthInfo {
  id: string;
  health: ModelHealthStatus;
  last_error?: string | null;
  since_secs?: number | null;
  context_window?: number | null;
}

export interface ModelConfigUI {
  id: string;
  provider: string;
  url: string;
  model: string;
  api_key?: string | null;
  keep_alive?: string | null;
  context_window?: number | null;
  tags?: string[];
  auth_mode?: string | null;
  reasoning_effort?: string | null;
}

export interface SessionInfo {
  id: string;
  repo_path: string;
  title: string;
  created_at: number;
  updated_at?: number; // last transcript activity (server-derived); created_at fallback
  creator?: string;         // "user" | "mission" | "skill"
  project?: string;         // full project path
  project_name?: string;    // short name (last path segment)
  skill?: string | null;    // bound skill name
  mission_id?: string | null; // set when the session is bound to a mission (creator "mission", or a user-opened attended session)
  cwd?: string;             // current working directory
  model_id?: string | null; // session-level model override
  permission_mode?: string | null; // effective permission mode (read/edit/admin)
}


export interface SkillToolParamDef {
  type: string;
  required: boolean;
  default?: unknown;
  description: string;
}

export interface SkillToolDef {
  name: string;
  description: string;
  cmd: string;
  args: Record<string, SkillToolParamDef>;
  returns?: string | null;
  timeout_ms: number;
}

export interface SkillAppConfig {
  launcher: 'web' | 'bash' | 'url';
  entry: string;
  /** `false` keeps an installed-but-unfinished app out of the tab bar. */
  list?: boolean;
  width?: number;
  height?: number;
}

export interface SkillPermission {
  mode: 'read' | 'edit' | 'admin';
  paths?: string[];
  warning?: string | null;
}

export interface SkillInfoFull {
  name: string;
  description: string;
  content: string;
  source: { type: 'Global' | 'Project' | 'Compat' };
  tool_defs: SkillToolDef[];
  user_invocable?: boolean;
  allowed_tools?: string[];
  argument_hint?: string | null;
  model?: string | null;
  trigger?: string | null;
  agent?: string | null;
  app?: SkillAppConfig | null;
  permission?: SkillPermission | null;
  /** Starter prompts shown as buttons above the skill's chat. */
  suggestions?: string[];
}

export interface SkillFileInfo {
  name: string;
  path: string;
  source: string;
}

export interface MarketplaceSkill {
  skill_id: string;
  name: string;
  url: string;
  description?: string | null;
  install_count: number;
  git_ref?: string | null;
  content?: string | null;
  updated_at?: string | null;
  source_registry?: string | null;
}

export interface BuiltInSkillInfo {
  name: string;
  description: string;
  installed: boolean;
}

export type ManagementTab = 'models' | 'agents' | 'skills' | 'tools' | 'mcp' | 'general' | 'phone' | 'mission' | 'storage' | 'room';

// --- Mission types (cron-based) ---

/** Permission block — mirrors SkillPermission. See doc/mission-spec.md.
 *  The backend returns per-path grants `{path, mode}` (no top-level
 *  mode). The editor displays paths bare and uses a separate dropdown
 *  for the default mode applied at save time. */
export interface MissionPermission {
  /** Per-path grants — each entry carries its own mode. */
  paths?: Array<{ path: string; mode: string }>;
  /** Warning shown in UI before enabling. */
  warning?: string | null;
}

export interface CronMission {
  id: string;
  name?: string | null;
  description?: string;
  schedule: string;
  enabled: boolean;
  agent_id: string;

  /** Markdown body — step-by-step instructions. */
  prompt: string;

  /** Working directory. */
  cwd?: string | null;
  /** Model override. */
  model?: string | null;
  /** Ordered user-turn messages seeded into the session at run start.
   *  Item 0 fires immediately; later items drain one-per-assistant-final-reply
   *  via the engine's kickoff queue. Empty → generic fallback line. */
  kickoff?: string[];

  /** Explicit tool allowlist. Empty → unrestricted. */
  allowed_tools?: string[];
  /** Permission block (mode + paths + warning). */
  permission?: MissionPermission | null;

  /** Legacy project field — kept for back-compat. Prefer `cwd`. */
  project?: string | null;

  /** The skill that ships this mission. Its file is the skill's; the user
   *  owns only on/off and the schedule. */
  skill?: string | null;

  created_at: number;
}

// --- Storage browser types ---

export interface StorageRoot {
  label: string;
  path: string;
}

export interface StorageEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size?: number | null;
  modified?: number | null;
  children_count?: number | null;
}

export interface PendingAskUser {
  questionId: string;
  agentId: string;
  questions: AskUserQuestion[];
}
