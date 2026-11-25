// The model picker's "Default" option names the model the session's next turn
// runs on when nothing is picked. The engine decides it (`GET
// /api/sessions/default-model`, the turn's own rule); the page only words it.
// Until the engine has answered — or when it has no model to name — the option
// says "Default" alone, never a guess from the routing list.

/** What the "Default" option reads: `model` is the engine's answer (an id). */
export function defaultOptionText(model: string | null | undefined, nameOf: (id: string) => string): string {
  return model ? `Default (${nameOf(model)})` : 'Default';
}

/** The model a turn runs on: the picked one, else the engine's default. */
export function modelInUse(picked: string | null | undefined, engineDefault: string | null | undefined): string | null {
  return picked || engineDefault || null;
}
