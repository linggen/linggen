/**
 * The events the engine pushes, one type per `kind`.
 *
 * The envelope and every `data` payload are generated from the Rust types
 * (`./generated`, see scripts/gen-types.sh); this file only says which payload
 * rides under which kind and phase. Control-channel pushes that the engine
 * builds outside the event mapping (`page_state`, `user_info`,
 * `yinyue_present`) are typed here by hand.
 */
import type { EventKind } from '../lib/eventKinds';
import type { UiEvent as WireEvent } from './generated/UiEvent';
import type { ActivityData } from './generated/ActivityData';
import type { AppLaunchedData } from './generated/AppLaunchedData';
import type { AskUserData } from './generated/AskUserData';
import type { AskUserQuestion } from './generated/AskUserQuestion';
import type { ContentBlockStartData } from './generated/ContentBlockStartData';
import type { ContentBlockUpdateData } from './generated/ContentBlockUpdateData';
import type { ContextUsageData } from './generated/ContextUsageData';
import type { DeviceTopicData } from './generated/DeviceTopicData';
import type { FollowupsData } from './generated/FollowupsData';
import type { MessageData } from './generated/MessageData';
import type { ModelFallbackData } from './generated/ModelFallbackData';
import type { NotificationPayload } from './generated/NotificationPayload';
import type { OutcomeData } from './generated/OutcomeData';
import type { PetExpressData } from './generated/PetExpressData';
import type { PetSpeakData } from './generated/PetSpeakData';
import type { PetVoiceData } from './generated/PetVoiceData';
import type { PlanUpdateData } from './generated/PlanUpdateData';
import type { QueueData } from './generated/QueueData';
import type { QueuedChatItem } from './generated/QueuedChatItem';
import type { ResyncData } from './generated/ResyncData';
import type { RoomChatData } from './generated/RoomChatData';
import type { SessionCreatedData } from './generated/SessionCreatedData';
import type { SkillSaveChangedData } from './generated/SkillSaveChangedData';
import type { QuestsChangedData } from './generated/QuestsChangedData';
import type { SubagentResultData } from './generated/SubagentResultData';
import type { SubagentSpawnedData } from './generated/SubagentSpawnedData';
import type { TextSegmentData } from './generated/TextSegmentData';
import type { TokenData } from './generated/TokenData';
import type { ToolProgressData } from './generated/ToolProgressData';
import type { TurnCompleteData } from './generated/TurnCompleteData';
import type { WidgetResolvedData } from './generated/WidgetResolvedData';
import type { WorkingFolderData } from './generated/WorkingFolderData';
import type { AgentInfo, AgentRunInfo, ModelInfo, SessionInfo, SkillInfo } from '../types';

type Envelope = Omit<WireEvent, 'kind' | 'phase' | 'data'>;

/** One event: its kind, the phase it may carry, and its payload. */
type Ev<K extends string, D, P extends string = string> = Envelope & {
  kind: K;
  phase?: P | null;
  data?: D | null;
};

/** `run` events: the phase says which payload. */
export type RunEvent =
  | Ev<'run', never, 'sync'>
  | Ev<'run', ResyncData, 'resync'>
  | Ev<'run', OutcomeData, 'outcome'>
  | Ev<'run', ContextUsageData, 'context_usage'>
  | Ev<'run', PlanUpdateData, 'plan_update'>
  | Ev<'run', SubagentSpawnedData, 'subagent_spawned'>
  | Ev<'run', SubagentResultData, 'subagent_result'>;

/** The `run` event of one phase. */
export type RunEventOf<P extends NonNullable<RunEvent['phase']>> = Extract<RunEvent, { phase?: P | null }>;

/** `content_block` events: `start` opens a block, `update` moves it on. */
export type ContentBlockEvent =
  | Ev<'content_block', ContentBlockStartData, 'start'>
  | Ev<'content_block', ContentBlockUpdateData, 'update'>;

/** The page_state push (server/rtc/page_state.rs `PageState`). Every field
 *  is optional: the server omits what didn't change or doesn't apply. */
export interface PageState {
  permission?: string;
  room_name?: string | null;
  room_enabled?: boolean | null;
  all_sessions?: SessionInfo[];
  models?: ModelInfo[];
  default_models?: string[];
  skills?: SkillInfo[];
  missions?: unknown[];
  pending_ask_user?: Array<{ question_id: string; agent_id?: string; questions?: AskUserQuestion[]; session_id?: string | null }>;
  busy_sessions?: Record<string, string>;
  agents?: AgentInfo[];
  agent_runs?: AgentRunInfo[];
  queued?: Array<{ agent_id?: string; items?: QueuedChatItem[] }>;
  sessions?: SessionInfo[];
  session_permission?: { effective_mode?: string } | null;
}

/** Who this peer is (server/rtc/peer `user_info_msg`), sent when the control
 *  channel opens. Older engines sent the user fields flat. */
export interface UserInfoUser {
  user_id?: string | null;
  user_type?: string | null;
  user_name?: string | null;
  avatar_url?: string | null;
}
export interface UserInfoData extends UserInfoUser {
  user?: UserInfoUser;
  room?: { room_name?: string | null; permission?: string | null; token_budget_daily?: number | null } | null;
}

export interface UiEventByKind {
  message: Ev<'message', MessageData>;
  token: Ev<'token', TokenData>;
  text_segment: Ev<'text_segment', TextSegmentData>;
  content_block: ContentBlockEvent;
  turn_complete: Ev<'turn_complete', TurnCompleteData>;
  followups: Ev<'followups', FollowupsData>;
  activity: Ev<'activity', ActivityData>;
  queue: Ev<'queue', QueueData>;
  run: RunEvent;
  ask_user: Ev<'ask_user', AskUserData>;
  widget_resolved: Ev<'widget_resolved', WidgetResolvedData>;
  notification: Ev<'notification', SessionCreatedData | NotificationPayload>;
  model_fallback: Ev<'model_fallback', ModelFallbackData>;
  tool_progress: Ev<'tool_progress', ToolProgressData>;
  app_launched: Ev<'app_launched', AppLaunchedData>;
  working_folder: Ev<'working_folder', WorkingFolderData>;
  pet_speak: Ev<'pet_speak', PetSpeakData>;
  pet_express: Ev<'pet_express', PetExpressData>;
  pet_voice: Ev<'pet_voice', PetVoiceData>;
  yinyue_present: Ev<'yinyue_present', { present?: boolean }>;
  page_state: Ev<'page_state', PageState>;
  user_info: Ev<'user_info', UserInfoData>;
  room_chat: Ev<'room_chat', RoomChatData>;
  device_topic: Ev<'device_topic', DeviceTopicData>;
  skill_save_changed: Ev<'skill_save_changed', SkillSaveChangedData>;
  quests_changed: Ev<'quests_changed', QuestsChangedData>;
}

/** Any event off the wire, discriminated by `kind`. */
export type UiEvent = UiEventByKind[EventKind];
/** The event of one kind. */
export type UiEventOf<K extends EventKind> = UiEventByKind[K];
