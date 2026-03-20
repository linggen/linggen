export interface SubagentToolStep {
  toolName: string;
  args: string;
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
}

export interface MessageSegment {
  type: 'text' | 'tools';
  text?: string;         // for 'text' segments
  entries?: string[];    // for 'tools' segments
}

/** Structured content block — Claude Code-style message model. */
export interface ContentBlock {
  type: 'text' | 'tool_use' | 'tool_result' | 'thinking';
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
}

export interface UiEvent {
  id: string;
  seq: number;
  rev: number;
  ts_ms: number;
  kind: 'message' | 'activity' | 'queue' | 'run' | 'token' | 'text_segment' | 'ask_user' | 'model_fallback' | 'content_block' | 'turn_complete' | 'app_launched' | 'tool_progress' | 'notification';
  phase?: string;
  text?: string;
  agent_id?: string;
  session_id?: string;
  project_root?: string;
  data?: any;
}

export interface QueuedChatItem {
  id: string;
  agent_id: string;
  session_id: string;
  preview: string;
  timestamp: number;
}

export interface FileEntry {
  name: string;
  isDir: boolean;
  path: string;
}

export interface WorkspaceState {
  active_task: [any, string] | null;
  user_stories: [any, string] | null;
  tasks: [any, string][];
  messages: [any, string][];
}

export interface ProjectInfo {
  path: string;
  name: string;
  added_at: number;
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
  server: { port: number };
  agent: { max_iters: number; write_safety_mode: string; tool_permission_mode: string; prompt_loop_breaker?: string | null };
  logging: { level?: string | null; directory?: string | null; retention_days?: number | null };
  agents: { id: string; spec_path: string; model?: string | null }[];
  routing?: { default_models?: string[]; default_policy?: string | null; auto_fallback?: boolean };
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
}

export interface AgentRunInfo {
  run_id: string;
  repo_path: string;
  session_id: string;
  agent_id: string;
  agent_kind?: string | null;
  parent_run_id?: string | null;
  status: 'running' | 'completed' | 'failed' | 'cancelled' | string;
  detail?: string | null;
  started_at: number;
  ended_at?: number | null;
}

export interface AgentRunSummary {
  run_id: string;
  status: string;
  started_at: number;
  ended_at?: number | null;
  child_count: number;
  timeline_events: number;
  last_event_at: number;
}

export interface AgentRunContextSummary {
  message_count: number;
  user_messages: number;
  agent_messages: number;
  system_messages: number;
  started_at: number;
  ended_at?: number | null;
}

export interface AgentRunContextMessage {
  repo_path: string;
  session_id: string;
  agent_id: string;
  from_id: string;
  to_id: string;
  content: string;
  timestamp: number;
  is_observation: boolean;
}

export interface AgentRunContextResponse {
  run: AgentRunInfo;
  summary: AgentRunContextSummary;
  messages?: AgentRunContextMessage[] | null;
}

export interface SkillToolParamDef {
  type: string;
  required: boolean;
  default?: any;
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
  width?: number;
  height?: number;
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

export type ManagementTab = 'models' | 'agents' | 'skills' | 'tools' | 'general' | 'mission' | 'storage';

// --- Mission types (cron-based) ---

export type MissionTab = 'list' | 'create' | 'edit' | 'runs' | 'agent';

export interface CronMission {
  id: string;
  name?: string | null;
  schedule: string;
  agent_id: string;
  prompt: string;
  model?: string | null;
  project?: string | null;
  /** Permission tier: "readonly", "standard", "full". Default: "full". */
  permission_tier?: string;
  enabled: boolean;
  created_at: number;
}

export interface MissionRunEntry {
  run_id: string;
  session_id?: string | null;
  triggered_at: number;
  status: string;
  skipped: boolean;
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

// Plan mode types
export type PlanStatus = 'planned' | 'approved' | 'executing' | 'completed' | 'rejected';

export interface PlanItem {
  id: string;
  title: string;
  status: string;
}

export interface Plan {
  summary: string;
  status: PlanStatus;
  plan_text: string;
  items?: PlanItem[];
}

// --- AskUser types ---

export interface AskUserOption {
  label: string;
  description?: string | null;
  preview?: string | null;
}

export interface AskUserQuestion {
  question: string;
  header: string;
  options: AskUserOption[];
  multi_select?: boolean;
}

export interface AskUserAnswer {
  question_index: number;
  selected: string[];
  custom_text?: string | null;
}

export interface PendingAskUser {
  questionId: string;
  agentId: string;
  questions: AskUserQuestion[];
}
