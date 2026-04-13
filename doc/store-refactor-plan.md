---
type: plan
---

# Store Refactor Plan

## Current State (4 stores, mixed responsibilities)

| Store | Lines | Issues |
|:--|:--|:--|
| `projectStore` | ~170 | Misnamed (sessions, not projects). Projects hidden. Mission/skill session flags mixed in. |
| `agentStore` | ~280 | Models + skills + agents + runs + ollama + cancellation. 5 no-op fetch functions. |
| `chatStore` | ~700 | Well-structured per-session buckets, but `fetchSessionState` does plan/askUser/permission restoration — too many responsibilities. |
| `uiStore` | ~180 | Navigation + toasts + overlays + plans + askUser + permissions + connection + user context — catch-all. |

## Target State (6 focused stores)

### `sessionStore` (rename from projectStore)
- `sessions`, `allSessions`, `activeSessionId`
- `isMissionSession`, `activeMissionId`, `isSkillSession`, `activeSkillName`
- `selectedProjectRoot` (workspace context)
- `agentTreesByProject`
- Actions: `createSession`, `removeSession`, `renameSession`, `setActiveSessionId`

### `serverStore` (extract from agentStore)
- `models`, `defaultModels`, `ollamaStatus`
- `skills`
- `agents`
- `agentRuns`, `cancellingRunIds`
- Actions: `cancelAgentRun`, `reloadSkills`, `reloadAgents`
- Actions: `toggleDefaultModel`, `setReasoningEffort` (config writes)
- No fetch functions — all data via page_state

### `chatStore` (keep, slim down)
- `messages`, `displayMessages`, `_messagesBySession`
- `sessionState`
- All message mutation methods (addMessage, upsertGenerating, appendToken, etc.)
- `fetchSessionState` — keep but only for admin users
- Remove plan/askUser restoration from fetchSessionState — move to eventDispatcher

### `uiStore` (keep, slim down)
- `currentPage`, `sidebarTab`
- `overlay`, `modelPickerOpen`, `showAgentSpecEditor`, `openApp`
- `selectedFileContent`, `selectedFilePath`
- `copyChatStatus`, `verboseMode`
- `toasts`
- `editingMission`, `missionRefreshKey`

### `userStore` (new — extract from uiStore)
- `userPermission` (admin/edit/read/chat/pending)
- `userRoomName`, `userTokenBudget`
- `connectionStatus`
- `sessionMode`, `sessionZone`
- Actions: `setUserInfo`, `setSessionMode`, `setSessionZone`

### `interactionStore` (new — extract from uiStore)
- `pendingAskUser`, `pendingPlan`, `pendingPlanAgentId`, `activePlan`
- `queuedMessages`
- Actions: `setPendingAskUser`, `setPendingPlan`, `setActivePlan`, `setQueuedMessages`

## Migration Strategy

1. **Phase 1**: Rename `projectStore` → `sessionStore`. Mechanical rename across all files.
2. **Phase 2**: Extract `userStore` from `uiStore`. Move permission/connection fields.
3. **Phase 3**: Extract `interactionStore` from `uiStore`. Move plan/askUser/queue.
4. **Phase 4**: Rename `agentStore` → `serverStore`. Remove dead no-op functions entirely.
5. **Phase 5**: Slim `chatStore`. Move plan restoration to eventDispatcher.

Each phase is independently shippable. Do one at a time, test, commit.

## Rules

- Each store has ONE clear responsibility
- No store imports another store's setState (use eventDispatcher for cross-store updates)
- All server-pushed data goes through eventDispatcher → store.setState
- Fetch functions only for data NOT in page_state
- No dead code — remove no-op functions, don't leave stubs
