// Sources and pictures in answers (doc/chat-spec.md). An answer refers to a
// tool result by its number — `[turn2search1]` a source, `[turn2image1]` a
// picture — and the session state hands out what each number resolves to
// (`refs`). This remark plugin turns the numbers into nodes the chat draws:
// a source becomes an inline `lg-ref` chip; the pictures of a paragraph or
// heading leave its words and follow it as one `lg-pictures` grid (inside a
// table cell, they stay in the cell). A number that resolves to nothing
// draws nothing — the components decide, from `refs`.

export interface AnswerRef {
  kind: 'source' | 'picture';
  url: string;
  title?: string;
  caption?: string;
  name?: string;
  price?: string;
  page?: string;
  /** A picture's drawable copy: the engine's kept file, or past the cap its address. */
  src?: string;
}

const REF = /\[(turn\d{1,6}(?:search|image)\d{1,6})\]/g;

export const isPictureId = (id: string): boolean => /^turn\d+image\d+$/.test(id);

/** The text of a streaming answer without a number still being written. */
export function hidePartialRef(text: string): string {
  return text.replace(/\[(?:t(?:u(?:r(?:n[a-z0-9]*)?)?)?)?$/, '');
}

interface MdNode {
  type: string;
  value?: string;
  children?: MdNode[];
  data?: { hName?: string; hProperties?: Record<string, string> };
}

const refNode = (id: string): MdNode => ({
  type: 'lgRef',
  data: { hName: 'lg-ref', hProperties: { dataRef: id } },
  children: [],
});

const picturesNode = (ids: string[]): MdNode => ({
  type: 'lgPictures',
  data: { hName: 'lg-pictures', hProperties: { dataRefs: ids.join(' ') } },
  children: [],
});

/** One text node split at its numbers: words and chips; pictures collected. */
function splitText(value: string, pictures: string[]): MdNode[] {
  const out: MdNode[] = [];
  let last = 0;
  for (const m of value.matchAll(REF)) {
    const id = m[1];
    let before = value.slice(last, m.index);
    if (isPictureId(id)) {
      before = before.replace(/[ \t]+$/, '');
      if (!pictures.includes(id)) pictures.push(id);
    }
    if (before) out.push({ type: 'text', value: before });
    if (!isPictureId(id)) out.push(refNode(id));
    last = (m.index ?? 0) + m[0].length;
  }
  const rest = value.slice(last);
  if (rest) out.push({ type: 'text', value: rest });
  return out;
}

/** Split every text below `node`, collecting picture ids into `pictures`. */
function splitDescendants(node: MdNode, pictures: string[]): void {
  if (!node.children) return;
  node.children = node.children.flatMap((child) => {
    if (child.type === 'text' && child.value && child.value.includes('[turn')) {
      return splitText(child.value, pictures);
    }
    splitDescendants(child, pictures);
    return [child];
  });
}

const FOLLOWED = new Set(['paragraph', 'heading']);

function transform(node: MdNode): void {
  if (!node.children) return;
  const next: MdNode[] = [];
  for (const child of node.children) {
    next.push(child);
    if (FOLLOWED.has(child.type)) {
      const pictures: string[] = [];
      splitDescendants(child, pictures);
      if (pictures.length) next.push(picturesNode(pictures));
      continue;
    }
    if (child.type === 'tableCell') {
      const pictures: string[] = [];
      splitDescendants(child, pictures);
      if (pictures.length) child.children = [...(child.children ?? []), picturesNode(pictures)];
      continue;
    }
    transform(child);
  }
  node.children = next;
}

/** The remark plugin. */
export function remarkAnswerRefs() {
  return (tree: MdNode) => transform(tree);
}

/** A source chip's label: the site, without `www.`. */
export function sourceLabel(ref: AnswerRef): string {
  try {
    return new URL(ref.url).hostname.replace(/^www\./, '');
  } catch {
    return ref.title || ref.url;
  }
}

/** What a picture's caption line says: a product's name and price, else its caption. */
export function pictureCaption(ref: AnswerRef): string {
  const product = [ref.name, ref.price].filter(Boolean).join(' · ');
  return product || ref.caption || '';
}
