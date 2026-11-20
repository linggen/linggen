type KeyTarget = {
  addEventListener(type: string, listener: EventListener, capture?: boolean): void;
  removeEventListener(type: string, listener: EventListener, capture?: boolean): void;
};

function topDialog(doc: Document) {
  return [...doc.querySelectorAll('[aria-modal="true"], dialog[open]')]
    .filter((element) => element.getClientRects().length > 0).at(-1);
}

export function watchTopDialogEscape(dialog: HTMLElement, close: () => void) {
  const doc = dialog.ownerDocument;
  return watchDialogEscape(doc.defaultView!, () => topDialog(doc) === dialog, close);
}

function editingTarget(event: KeyboardEvent) {
  const elements = event.composedPath().filter((node): node is Element => node instanceof Element);
  const field = 'input:not([type="button"], [type="submit"], [type="reset"], [type="image"]), textarea, select, [role="textbox"]';
  // isContentEditable includes inherited editability and honours explicit
  // contenteditable="false" islands inside an editor.
  const target = elements.find((node): node is HTMLElement => node instanceof HTMLElement);
  return target?.isContentEditable || elements.some((node) => node.matches(field));
}

export function watchPageEscape(doc: Document, close: () => void) {
  return watchDialogEscape(doc.defaultView!, (key) => !editingTarget(key) && !topDialog(doc), close);
}

export function watchDialogEscape(target: KeyTarget, isTop: (key: KeyboardEvent) => boolean, close: () => void) {
  let offered: Event | undefined;
  const capture = (event: Event) => {
    const key = event as KeyboardEvent;
    // Snapshot before descendants can synchronously remove the top dialog.
    if (key.key === 'Escape' && !key.repeat && !key.isComposing && key.keyCode !== 229 && isTop(key)) offered = event;
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
