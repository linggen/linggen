/**
 * What a key does in the confirm dialog: Escape cancels; Enter confirms —
 * unless a button has the focus, which Enter presses itself (Cancel then
 * cancels). Nothing while an input method is composing. Pure, so node
 * tests can hold it.
 */
export function confirmKey(
  key: string,
  composing: boolean,
  onButton: boolean,
): 'confirm' | 'cancel' | null {
  if (composing) return null;
  if (key === 'Escape') return 'cancel';
  if (key === 'Enter' && !onButton) return 'confirm';
  return null;
}
