type KeyTarget = {
  addEventListener(type: string, listener: EventListener, capture?: boolean): void;
  removeEventListener(type: string, listener: EventListener, capture?: boolean): void;
};

export function watchTopDialogEscape(dialog: HTMLElement, close: () => void) {
  const doc = dialog.ownerDocument;
  const isTop = () => [...doc.querySelectorAll('[aria-modal="true"], dialog[open]')]
    .filter((element) => element.getClientRects().length > 0).at(-1) === dialog;
  return watchDialogEscape(doc.defaultView!, isTop, close);
}

export function watchDialogEscape(target: KeyTarget, isTop: () => boolean, close: () => void) {
  let offered: Event | undefined;
  const capture = (event: Event) => {
    const key = event as KeyboardEvent;
    // Snapshot before descendants can synchronously remove the top dialog.
    if (key.key === 'Escape' && !key.repeat && !key.isComposing && key.keyCode !== 229 && isTop()) offered = event;
  };
  const bubble = (event: Event) => {
    // Editors and menus own Escape when they handle it (preventDefault).
    if (event !== offered || event.defaultPrevented) return;
    event.preventDefault();
    close();
  };
  target.addEventListener('keydown', capture, true);
  target.addEventListener('keydown', bubble);
  return () => {
    target.removeEventListener('keydown', capture, true);
    target.removeEventListener('keydown', bubble);
  };
}
