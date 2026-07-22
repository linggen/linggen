/**
 * Handler registry — a table keyed by canonical `EventKind` that maps each
 * event kind to the function that applies it to the stores.
 *
 * Adding a new kind is a one-line change:
 *   1. Add the kind string to `EVENT_KINDS` in `../eventKinds.ts`.
 *   2. Add a `<kind>: handler` entry here.
 * TypeScript enforces both: missing entries fail to compile because the map
 * is typed `Record<EventKind, EventHandler>`.
 */
import type { UiEvent } from '../../types';
import type { EventKind } from '../eventKinds';

import { handleRun } from './run';
import { handleActivity } from './activity';
import {
  handleTextSegment,
  handleToken,
  handleMessage,
  handleContentBlock,
  handleTurnComplete,
  handleToolProgress,
} from './chat';
import {
  handleAskUser,
  handleWidgetResolved,
  handleQueue,
  handleModelFallback,
} from './interactive';
import { handlePageState } from './pageState';
import {
  handleNotification,
  handleAppLaunched,
  handleWorkingFolder,
  handleUserInfo,
  handleRoomChat,
  handleDeviceTopic,
} from './misc';
import { handlePetSpeak, handlePetExpress, handleYinyuePresent } from './yinyue';

export type EventHandler = (item: UiEvent) => void;

export const eventHandlers: Record<EventKind, EventHandler> = {
  // Chat / streaming
  message: handleMessage,
  token: handleToken,
  text_segment: handleTextSegment,
  content_block: handleContentBlock,
  turn_complete: handleTurnComplete,

  // Activity / lifecycle
  activity: handleActivity,
  queue: handleQueue,
  run: handleRun,

  // Interactive widgets
  ask_user: handleAskUser,
  widget_resolved: handleWidgetResolved,

  // Notifications / fallbacks
  notification: handleNotification,
  model_fallback: handleModelFallback,
  tool_progress: handleToolProgress,
  app_launched: handleAppLaunched,
  working_folder: handleWorkingFolder,

  // Control-channel pushes
  page_state: handlePageState,
  user_info: handleUserInfo,
  room_chat: handleRoomChat,
  device_topic: handleDeviceTopic,

  // Pet (Yinyue today)
  pet_speak: handlePetSpeak,
  pet_express: handlePetExpress,
  yinyue_present: handleYinyuePresent,
};

export { handleAskUser };
