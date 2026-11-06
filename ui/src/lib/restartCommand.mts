// A restart is a standalone command, never a prefix or an attachment.
export function isRestartCommand(text: string, images?: readonly string[]): boolean {
  return text.trim() === '/restart' && !images?.length;
}

export const restartSuggestion = {
  key: 'cmd-restart', label: '/restart', description: 'Restart the engine and reconnect', isBuiltin: true,
};

/** The request is accepted before the engine disconnects; reconnection is handled by the transport. */
export async function requestRestart(post: (path: string) => Promise<unknown>): Promise<void> {
  await post('/api/restart/current');
}
