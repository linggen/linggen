/** Translate a placeholder, never a field's value. */
export function translatePlaceholder(value: string, optedOut: boolean, translate: (text: string) => string | null): string | null {
  return optedOut ? null : translate(value);
}
