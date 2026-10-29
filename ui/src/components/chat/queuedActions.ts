/**
 * Remove and Steer on a queued message. The row hides at the click and
 * comes back when the engine never heard it, so a dead connection never
 * looks like a deletion.
 */
import type { QueuedChatItem } from '../../types/generated/QueuedChatItem';
import { useInteractionStore } from '../../stores/interactionStore';
import { useUiStore } from '../../stores/uiStore';
import { connection } from '../../lib/transport';
import { ApiError, apiErrorMessage } from '../../lib/api';
import { sessionApi } from '../../lib/endpoints';
import { restoreQueued, withoutQueued } from '../../lib/queuedList.mts';

const NOT_CONNECTED = 'Not connected to Linggen. The message is still queued.';

function toastError(message: string): void {
  useUiStore.getState().addToast({ message, variant: 'error' });
}

/** Hide `item`, run `send`; put it back when the request never reached the
 *  engine, or when the engine refused and `refusalKeeps` (its queue is
 *  unchanged then). Null when the action did not happen. */
async function hideWhile<T>(item: QueuedChatItem, send: () => Promise<T>, refusalKeeps: boolean): Promise<T | null> {
  if (!connection.isOpen()) {
    toastError(NOT_CONNECTED);
    return null;
  }
  const store = useInteractionStore.getState();
  const { list, at } = withoutQueued(store.queuedMessages, item.id);
  store.setQueuedMessages(list);
  try {
    return await send();
  } catch (error) {
    const refused = error instanceof ApiError;
    if (!refused || refusalKeeps) {
      useInteractionStore.getState().setQueuedMessages((now) => restoreQueued(now, item, at));
    }
    toastError(refused ? apiErrorMessage(error) : NOT_CONNECTED);
    return null;
  }
}

/** Take one message off the engine's queue (it never runs); resolves to its
 *  text and pictures, or null when it was not taken. A refused remove sent
 *  the engine's queue along, so the list is already right. */
export function takeQueued(sessionId: string, agentId: string, item: QueuedChatItem) {
  return hideWhile(item, () => sessionApi.removeQueued(sessionId, agentId, item.id), false);
}

/** Claude sessions: send one queued message into the running turn now. */
export function steerQueued(item: QueuedChatItem): void {
  void hideWhile(item, () => sessionApi.steerQueued(item.id), true);
}
