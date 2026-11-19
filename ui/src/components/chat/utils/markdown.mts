export const hashText = (text: string) => {
  let hash = 0;
  for (let i = 0; i < text.length; i += 1) {
    hash = (hash * 31 + text.charCodeAt(i)) | 0;
  }
  return Math.abs(hash).toString(36);
};

/** A line opening (or closing) a code fence: up to three spaces, then three
 *  or more backticks or tildes. */
const FENCE = /^ {0,3}(`{3,}|~{3,})/;

/** The text as prose and fenced code, in order. A fence still open at the
 *  end (a reply streaming) is code to the end. */
function splitFences(text: string): { code: boolean; text: string }[] {
  const parts: { code: boolean; text: string }[] = [];
  let open: string | null = null;
  let buf: string[] = [];
  const push = (code: boolean) => {
    if (buf.length) parts.push({ code, text: buf.join('\n') });
    buf = [];
  };
  for (const line of text.split('\n')) {
    const fence = line.match(FENCE)?.[1];
    if (!open && fence) {
      push(false);
      open = fence;
      buf.push(line);
    } else if (open && fence && fence[0] === open[0] && fence.length >= open.length && !line.trim().slice(fence.length)) {
      buf.push(line);
      push(true);
      open = null;
    } else {
      buf.push(line);
    }
  }
  push(open !== null);
  return parts;
}

/** The pre-parse tidy-up of prose: comments out, a heading or list item run
 *  into the line before it onto its own line. */
function tidyProse(text: string): string {
  return text
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/\s+(#{1,6}\s)/g, '\n\n$1')
    .replace(/\s+(\d+\.\s)/g, '\n$1')
    .replace(/\s+(-\s)/g, '\n$1')
    .replace(/\n{3,}/g, '\n\n');
}

/** Tidies the prose for the markdown parser; nothing inside a code fence is
 *  touched, so code shows and copies as it was sent. */
export function normalizeMarkdownish(text: string): string {
  return splitFences(text)
    .map((part) => (part.code ? part.text : tidyProse(part.text)))
    .join('\n')
    .trim();
}
