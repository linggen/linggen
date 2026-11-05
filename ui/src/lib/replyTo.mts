// A tap on a speaker's name above their line (ChatMessageList) → whom the composer's chip names
// (ChatInput), the Mac's half of the phone's tap on a face (Hanli 11-05: 「点Yinyue头像, 自动发给Yinyue」).

/** The agent a reply-tap addresses: the tapped speaker when they are one of the chat's addressable
 *  main agents and there is more than one to tell apart; else none (the chip is left as it is). */
export function replyAddressee(tapped: string | null | undefined, agents: { name: string }[]): string | null {
  const id = (tapped ?? '').trim().toLowerCase();
  if (!id || agents.length < 2) return null;
  return agents.some((a) => a.name.toLowerCase() === id) ? id : null;
}
