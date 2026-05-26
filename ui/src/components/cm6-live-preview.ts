/**
 * Live Preview Plugin for CodeMirror 6
 *
 * Hides markdown syntax and shows rendered content inline,
 * similar to Obsidian's Live Preview mode.
 *
 * When cursor is on a line, syntax is shown; when cursor moves away,
 * the markdown is rendered.
 *
 * CodeMirror 6 live preview extension with theme
 * adjustments for Tailwind v4 UI.
 */

import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from '@codemirror/view';
import { syntaxTree } from '@codemirror/language';
import {
  RangeSetBuilder,
  StateField,
  type EditorState,
} from '@codemirror/state';

// Lazy mermaid import to avoid blocking
let mermaidInstance: typeof import('mermaid').default | null = null;
let mermaidInitialized = false;

async function getMermaid() {
  if (!mermaidInstance) {
    try {
      const mermaidModule = await import('mermaid');
      mermaidInstance = mermaidModule.default;
      if (!mermaidInitialized) {
        const isDark =
          document.documentElement.classList.contains('dark') ||
          window.matchMedia?.('(prefers-color-scheme: dark)').matches;
        mermaidInstance.initialize({
          startOnLoad: false,
          theme: isDark ? 'dark' : 'default',
          securityLevel: 'loose',
        });
        mermaidInitialized = true;
      }
    } catch (err) {
      console.error('Failed to load mermaid:', err);
      return null;
    }
  }
  return mermaidInstance;
}

// === Widget Classes for replaced content ===

// Track which mermaid blocks are in "edit raw text" mode.
const mermaidEditBlocks = new Set<number>();

class MermaidWidget extends WidgetType {
  private code: string;
  private id: string;
  private blockPos: number;

  constructor(code: string, blockPos: number) {
    super();
    this.code = code;
    this.blockPos = blockPos;
    this.id = `mermaid-${Math.random().toString(36).substr(2, 9)}`;
  }

  toDOM(view: EditorView) {
    const container = document.createElement('div');
    container.className = 'cm-mermaid-container';
    container.style.position = 'relative';
    container.style.margin = '8px 0';

    const editBtn = document.createElement('button');
    editBtn.style.cssText =
      'position:absolute;top:8px;right:8px;background:rgba(59,130,246,0.8);border:none;color:white;padding:2px 8px;border-radius:4px;cursor:pointer;font-size:12px;z-index:10;opacity:0;transition:opacity 0.2s';
    editBtn.innerHTML = '&lt;/&gt;';
    editBtn.title = 'Edit this block';

    container.addEventListener('mouseenter', () => {
      editBtn.style.opacity = '1';
    });
    container.addEventListener('mouseleave', () => {
      editBtn.style.opacity = '0';
    });

    editBtn.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      mermaidEditBlocks.add(this.blockPos);
      view.dispatch({
        selection: { anchor: this.blockPos },
        scrollIntoView: true,
      });
      view.focus();
    });

    const diagramContainer = document.createElement('div');
    diagramContainer.style.cssText =
      'display:flex;justify-content:center;padding:16px;background:rgba(0,0,0,0.08);border-radius:8px;min-height:80px';
    diagramContainer.innerHTML =
      '<div style="color:#94a3b8;padding:16px">Loading diagram...</div>';

    container.appendChild(editBtn);
    container.appendChild(diagramContainer);

    this.renderMermaid(diagramContainer);

    return container;
  }

  private async renderMermaid(container: HTMLElement) {
    try {
      const mermaid = await getMermaid();
      if (!mermaid) {
        container.innerHTML =
          '<div style="color:#ef4444;padding:8px">Mermaid not available</div>';
        return;
      }
      const cleanCode = this.code.trim();
      const { svg } = await mermaid.render(this.id, cleanCode);
      container.innerHTML = svg;
    } catch (err) {
      container.innerHTML = `<div style="color:#ef4444;padding:8px">Mermaid Error: ${err instanceof Error ? err.message : String(err)}</div>`;
    }
  }

  eq(other: MermaidWidget) {
    return this.code === other.code && this.blockPos === other.blockPos;
  }

  ignoreEvent(_event: Event) {
    return false;
  }
}

class HorizontalRuleWidget extends WidgetType {
  toDOM() {
    const hr = document.createElement('hr');
    hr.className = 'cm-hr-widget';
    return hr;
  }
}

class CheckboxWidget extends WidgetType {
  private checked: boolean;

  constructor(checked: boolean) {
    super();
    this.checked = checked;
  }

  toDOM() {
    const span = document.createElement('span');
    span.className = `cm-checkbox-widget ${this.checked ? 'checked' : ''}`;
    span.textContent = this.checked ? '\u2611' : '\u2610';
    return span;
  }
}

class BulletWidget extends WidgetType {
  toDOM() {
    const span = document.createElement('span');
    span.className = 'cm-list-bullet';
    span.textContent = '\u2022 ';
    return span;
  }
}

/** Renders a GFM table as a real HTML <table>. The widget receives the raw
 *  markdown lines (including the header row, delimiter row, and body rows);
 *  it splits on `|`, handles alignment from the delimiter row, and emits
 *  thead/tbody. The original markdown lines underneath are hidden via
 *  `cm-table-hidden-line` so the rendered table appears in their place.
 *  When the cursor enters any line of the table, the entire block falls
 *  back to raw markdown \u2014 same UX as headers/code blocks. */
class TableWidget extends WidgetType {
  private raw: string;

  constructor(raw: string) {
    super();
    this.raw = raw;
  }

  toDOM() {
    const wrapper = document.createElement('div');
    wrapper.className = 'cm-table-widget';
    const table = document.createElement('table');
    table.className = 'cm-rendered-table';

    const lines = this.raw.split('\n').filter((l) => l.trim().length > 0);
    if (lines.length < 2) {
      wrapper.appendChild(table);
      return wrapper;
    }

    const splitRow = (line: string): string[] => {
      let s = line.trim();
      if (s.startsWith('|')) s = s.slice(1);
      if (s.endsWith('|')) s = s.slice(0, -1);
      // Split on unescaped `|`.
      return s.split(/(?<!\\)\|/).map((c) => c.trim().replace(/\\\|/g, '|'));
    };

    const headerCells = splitRow(lines[0]);
    const delimCells = splitRow(lines[1]);
    const aligns: ('left' | 'center' | 'right' | null)[] = delimCells.map((c) => {
      const left = c.startsWith(':');
      const right = c.endsWith(':');
      if (left && right) return 'center';
      if (right) return 'right';
      if (left) return 'left';
      return null;
    });

    const thead = document.createElement('thead');
    const headerTr = document.createElement('tr');
    headerCells.forEach((cell, i) => {
      const th = document.createElement('th');
      th.textContent = cell;
      const a = aligns[i];
      if (a) th.style.textAlign = a;
      headerTr.appendChild(th);
    });
    thead.appendChild(headerTr);
    table.appendChild(thead);

    const tbody = document.createElement('tbody');
    for (let r = 2; r < lines.length; r++) {
      const cells = splitRow(lines[r]);
      const tr = document.createElement('tr');
      cells.forEach((cell, i) => {
        const td = document.createElement('td');
        td.textContent = cell;
        const a = aligns[i];
        if (a) td.style.textAlign = a;
        tr.appendChild(td);
      });
      tbody.appendChild(tr);
    }
    table.appendChild(tbody);
    wrapper.appendChild(table);
    return wrapper;
  }

  eq(other: TableWidget) {
    return this.raw === other.raw;
  }

  ignoreEvent() {
    return false;
  }
}

// === Decoration Classes ===

const hiddenMarkDecoration = Decoration.mark({ class: 'cm-hidden-syntax' });
const boldDecoration = Decoration.mark({ class: 'cm-rendered-strong' });
const italicDecoration = Decoration.mark({ class: 'cm-rendered-emphasis' });
const strikeDecoration = Decoration.mark({ class: 'cm-rendered-strike' });
const linkDecoration = Decoration.mark({ class: 'cm-rendered-link' });
const codeDecoration = Decoration.mark({ class: 'cm-rendered-code' });
const blockquoteDecoration = Decoration.line({ class: 'cm-blockquote-line' });

// === Helper functions ===

function getActiveLines(view: EditorView): Set<number> {
  const activeLines = new Set<number>();
  for (const range of view.state.selection.ranges) {
    const startLine = view.state.doc.lineAt(range.from).number;
    const endLine = view.state.doc.lineAt(range.to).number;
    for (let i = startLine; i <= endLine; i++) {
      activeLines.add(i);
    }
  }
  return activeLines;
}

/** Byte offset of the END of the YAML frontmatter (the position right after
 *  the closing `---`). Returns 0 when the document doesn't start with a
 *  frontmatter delimiter. Lezer's markdown grammar parses the whole file as
 *  markdown, so YAML comments inside the frontmatter would otherwise be
 *  treated as H1 headings — every decoration rule must check this range and
 *  bail out for nodes inside it. */
function computeFrontmatterEnd(doc: import('@codemirror/state').Text): number {
  if (doc.lines < 2) return 0;
  if (doc.line(1).text.trim() !== '---') return 0;
  for (let i = 2; i <= doc.lines; i++) {
    if (doc.line(i).text.trim() === '---') return doc.line(i).to;
  }
  return 0;
}

// === The main ViewPlugin ===

export const livePreviewPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;

    constructor(view: EditorView) {
      this.decorations = this.buildDecorations(view);
    }

    update(update: ViewUpdate) {
      if (update.docChanged || update.selectionSet || update.viewportChanged) {
        this.decorations = this.buildDecorations(update.view);
      }
    }

    buildDecorations(view: EditorView): DecorationSet {
      const activeLines = getActiveLines(view);
      const doc = view.state.doc;

      // YAML frontmatter region — the Lezer markdown grammar doesn't know
      // about frontmatter, so `# ...` comments inside the `---…---` block
      // get parsed as H1 headings. Compute the byte range of the
      // frontmatter once and skip every live-preview rule for nodes that
      // start inside it.
      const frontmatterEnd = computeFrontmatterEnd(doc);
      const frontmatterEndLine = frontmatterEnd > 0
        ? doc.lineAt(frontmatterEnd).number
        : 0;

      const decorations: {
        from: number;
        to: number;
        decoration: Decoration;
      }[] = [];

      // Tag every line inside the frontmatter so CSS can override the
      // header styling that CodeMirror's markdown extension applies
      // unconditionally (it adds `cm-header-line cm-header-1` to lines
      // starting with `#`, which inside YAML is just a comment).
      if (frontmatterEndLine > 0) {
        for (let n = 1; n <= frontmatterEndLine; n++) {
          const ln = doc.line(n);
          decorations.push({
            from: ln.from,
            to: ln.from,
            decoration: Decoration.line({ class: 'cm-frontmatter-line' }),
          });
        }
      }

      // === Standard markdown live preview decorations ===
      for (const { from, to } of view.visibleRanges) {
        syntaxTree(view.state).iterate({
          from,
          to,
          enter: (node) => {
            // Skip anything that lives inside the YAML frontmatter.
            if (node.from < frontmatterEnd) return;

            const line = doc.lineAt(node.from);
            const isActiveLine = activeLines.has(line.number);

            // Show raw markdown on active lines
            if (isActiveLine) return;

            const nodeType = node.name;

            // Headers — hide # marks
            if (
              nodeType.startsWith('ATXHeading') ||
              nodeType === 'HeaderMark'
            ) {
              if (nodeType === 'HeaderMark') {
                decorations.push({
                  from: node.from,
                  to: node.to + 1,
                  decoration: hiddenMarkDecoration,
                });
              }
            }

            // Bold **text** or __text__
            if (nodeType === 'StrongEmphasis') {
              const text = doc.sliceString(node.from, node.to);
              const marker = text.startsWith('**') ? '**' : '__';
              decorations.push({
                from: node.from,
                to: node.from + marker.length,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.to - marker.length,
                to: node.to,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.from + marker.length,
                to: node.to - marker.length,
                decoration: boldDecoration,
              });
            }

            // Italic *text* or _text_
            if (nodeType === 'Emphasis') {
              const text = doc.sliceString(node.from, node.to);
              const marker = text.startsWith('*') ? '*' : '_';
              decorations.push({
                from: node.from,
                to: node.from + marker.length,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.to - marker.length,
                to: node.to,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.from + marker.length,
                to: node.to - marker.length,
                decoration: italicDecoration,
              });
            }

            // Strikethrough ~~text~~
            if (nodeType === 'Strikethrough') {
              decorations.push({
                from: node.from,
                to: node.from + 2,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.to - 2,
                to: node.to,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.from + 2,
                to: node.to - 2,
                decoration: strikeDecoration,
              });
            }

            // Inline code `code`
            if (nodeType === 'InlineCode') {
              decorations.push({
                from: node.from,
                to: node.from + 1,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.to - 1,
                to: node.to,
                decoration: hiddenMarkDecoration,
              });
              decorations.push({
                from: node.from + 1,
                to: node.to - 1,
                decoration: codeDecoration,
              });
            }

            // Links [text](url)
            if (nodeType === 'Link') {
              const text = doc.sliceString(node.from, node.to);
              const linkMatch = text.match(/^\[([^\]]*)\]\(([^)]*)\)$/);
              if (linkMatch) {
                const textStart = node.from + 1;
                const textEnd = node.from + 1 + linkMatch[1].length;
                decorations.push({
                  from: node.from,
                  to: node.from + 1,
                  decoration: hiddenMarkDecoration,
                });
                decorations.push({
                  from: textEnd,
                  to: node.to,
                  decoration: hiddenMarkDecoration,
                });
                decorations.push({
                  from: textStart,
                  to: textEnd,
                  decoration: linkDecoration,
                });
              }
            }

            // Blockquotes > — hide QuoteMark
            if (nodeType === 'QuoteMark') {
              decorations.push({
                from: node.from,
                to: node.to + 1,
                decoration: hiddenMarkDecoration,
              });
            }

            if (nodeType === 'Blockquote') {
              decorations.push({
                from: line.from,
                to: line.from,
                decoration: blockquoteDecoration,
              });
            }

            // Horizontal rule ---
            if (nodeType === 'HorizontalRule') {
              decorations.push({
                from: node.from,
                to: node.to,
                decoration: Decoration.replace({
                  widget: new HorizontalRuleWidget(),
                }),
              });
            }

            // List markers
            if (nodeType === 'ListMark') {
              decorations.push({
                from: node.from,
                to: node.to + 1,
                decoration: Decoration.replace({
                  widget: new BulletWidget(),
                }),
              });
            }

            // Task list checkboxes
            if (nodeType === 'TaskMarker') {
              const text = doc.sliceString(node.from, node.to);
              const isChecked = text.includes('x') || text.includes('X');
              decorations.push({
                from: node.from,
                to: node.to,
                decoration: Decoration.replace({
                  widget: new CheckboxWidget(isChecked),
                }),
              });
            }

            // Fenced code blocks ```lang\n...code...\n```
            // Hide the opening fence line (```lang) and closing fence line
            // (```) when no cursor sits in the block. Style the code body
            // lines as a code-block. Mermaid blocks are handled separately
            // below (they get a full Mermaid widget), so we skip them here.
            if (nodeType === 'FencedCode') {
              const blockStartLine = doc.lineAt(node.from).number;
              const blockEndLine = doc.lineAt(node.to).number;
              if (blockStartLine === blockEndLine) return; // malformed/empty

              let blockHasActive = false;
              for (let n = blockStartLine; n <= blockEndLine; n++) {
                if (activeLines.has(n)) {
                  blockHasActive = true;
                  break;
                }
              }
              if (blockHasActive) return;

              // Detect mermaid (handled by the dedicated widget below).
              const firstLineText = doc.line(blockStartLine).text.trim();
              if (firstLineText.startsWith('```mermaid')) return;

              // Hide the opening fence line entirely.
              decorations.push({
                from: doc.line(blockStartLine).from,
                to: doc.line(blockStartLine).from,
                decoration: Decoration.line({
                  class: 'cm-codefence-hidden-line',
                }),
              });
              // Hide the closing fence line (only if it's a separate line).
              if (blockEndLine > blockStartLine) {
                decorations.push({
                  from: doc.line(blockEndLine).from,
                  to: doc.line(blockEndLine).from,
                  decoration: Decoration.line({
                    class: 'cm-codefence-hidden-line',
                  }),
                });
              }
              // Style intermediate code body lines.
              const bodyStart = blockStartLine + 1;
              const bodyEnd = blockEndLine - 1;
              for (let n = bodyStart; n <= bodyEnd; n++) {
                const ln = doc.line(n);
                let cls = 'cm-code-block-line';
                if (n === bodyStart) cls += ' cm-code-block-line-first';
                if (n === bodyEnd) cls += ' cm-code-block-line-last';
                decorations.push({
                  from: ln.from,
                  to: ln.from,
                  decoration: Decoration.line({ class: cls }),
                });
              }
            }

            // GFM tables — rendering a multi-line table widget requires a
            // StateField (ViewPlugin replace decorations can't cross line
            // breaks). Tables stay as raw markdown for now; readable as
            // text. Revisit via StateField if we want true rendering.
          },
        });
      }

      // === Mermaid fenced code blocks ===
      const fullText = doc.toString();
      const mermaidRegex = /```mermaid\s*\n([\s\S]*?)```/g;
      let match: RegExpExecArray | null;

      while ((match = mermaidRegex.exec(fullText)) !== null) {
        const blockCode = match[1];
        const blockStart = match.index;
        const blockEnd = match.index + match[0].length;

        // Skip mermaid blocks parsed from inside the YAML frontmatter
        // (very unlikely, but the regex doesn't know about frontmatter).
        if (blockStart < frontmatterEnd) continue;

        if (mermaidEditBlocks.has(blockStart)) {
          let stillEditing = false;
          for (const range of view.state.selection.ranges) {
            if (range.from <= blockEnd && range.to >= blockStart) {
              stillEditing = true;
              break;
            }
          }
          if (!stillEditing) {
            mermaidEditBlocks.delete(blockStart);
          } else {
            continue;
          }
        }

        decorations.push({
          from: blockStart,
          to: blockStart,
          decoration: Decoration.widget({
            widget: new MermaidWidget(blockCode, blockStart),
          }),
        });

        const startLine = doc.lineAt(blockStart).number;
        const endLine = doc.lineAt(blockEnd).number;
        for (let lineNo = startLine; lineNo <= endLine; lineNo++) {
          const line = doc.line(lineNo);
          decorations.push({
            from: line.from,
            to: line.from,
            decoration: Decoration.line({
              class: 'cm-mermaid-hidden-line',
            }),
          });
        }
      }

      // Sort decorations by position
      decorations.sort((a, b) => a.from - b.from || a.to - b.to);

      const builder = new RangeSetBuilder<Decoration>();
      for (const { from, to, decoration } of decorations) {
        try {
          builder.add(from, to, decoration);
        } catch {
          // Skip invalid ranges
        }
      }

      return builder.finish();
    }
  },
  {
    decorations: (v) => v.decorations,
  }
);

// === Tables — StateField for block decorations ===
//
// CodeMirror's ViewPlugin layer only accepts inline decorations; block
// widgets and decorations that replace line breaks must come from a
// StateField. Tables are inherently multi-line, so the table renderer
// lives here, separate from the rest of the live-preview decorations.
//
// One field, one job: walk the syntax tree, find `Table` nodes that are
// outside the YAML frontmatter and outside the user's current selection,
// and replace each one with a TableWidget rendered as a block.

function buildTableDecorations(state: EditorState): DecorationSet {
  const doc = state.doc;
  const frontmatterEnd = computeFrontmatterEnd(doc);

  // Collect selection lines so we can fall back to raw markdown when the
  // user clicks into the table.
  const activeLines = new Set<number>();
  for (const range of state.selection.ranges) {
    const a = doc.lineAt(range.from).number;
    const b = doc.lineAt(range.to).number;
    for (let n = a; n <= b; n++) activeLines.add(n);
  }

  const builder = new RangeSetBuilder<Decoration>();
  syntaxTree(state).iterate({
    enter: (node) => {
      if (node.name !== 'Table') return;
      if (node.from < frontmatterEnd) return;

      const tStart = doc.lineAt(node.from).number;
      const tEnd = doc.lineAt(node.to).number;
      for (let n = tStart; n <= tEnd; n++) {
        if (activeLines.has(n)) return;
      }

      const raw = doc.sliceString(node.from, node.to);
      builder.add(
        node.from,
        node.to,
        Decoration.replace({
          widget: new TableWidget(raw),
          block: true,
        }),
      );
    },
  });
  return builder.finish();
}

export const livePreviewTableField = StateField.define<DecorationSet>({
  create(state) {
    return buildTableDecorations(state);
  },
  update(value, tr) {
    if (!tr.docChanged && !tr.selection) return value;
    return buildTableDecorations(tr.state);
  },
  provide: (f) => EditorView.decorations.from(f),
});

// === Theme for live preview elements ===

export const livePreviewTheme = EditorView.theme({
  '.cm-hidden-syntax': {
    fontSize: '0',
    width: '0',
    display: 'none',
  },

  // Headers — rendered via CM6 markdown language support
  '.cm-header-line.cm-header-1': {
    fontSize: '1.8em',
    fontWeight: 'bold',
    lineHeight: '1.3',
    color: '#e2e8f0',
  },
  '.cm-header-line.cm-header-2': {
    fontSize: '1.5em',
    fontWeight: 'bold',
    lineHeight: '1.3',
    color: '#e2e8f0',
  },
  '.cm-header-line.cm-header-3': {
    fontSize: '1.3em',
    fontWeight: 'bold',
    lineHeight: '1.3',
    color: '#e2e8f0',
  },

  '.cm-rendered-strong': {
    fontWeight: 'bold',
    color: '#e2e8f0',
  },
  '.cm-rendered-emphasis': {
    fontStyle: 'italic',
    color: '#94a3b8',
  },
  '.cm-rendered-strike': {
    textDecoration: 'line-through',
    color: '#64748b',
  },
  '.cm-rendered-link': {
    color: '#60a5fa',
    textDecoration: 'underline',
    cursor: 'pointer',
  },
  '.cm-rendered-code': {
    backgroundColor: 'rgba(59,130,246,0.15)',
    color: '#60a5fa',
    padding: '2px 6px',
    borderRadius: '4px',
    fontFamily:
      'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
    fontSize: '0.9em',
  },
  '.cm-blockquote-line': {
    borderLeft: '3px solid #3b82f6',
    paddingLeft: '12px',
    color: '#64748b',
    fontStyle: 'italic',
  },
  '.cm-list-bullet': {
    color: '#3b82f6',
    fontWeight: 'bold',
    marginRight: '8px',
  },
  '.cm-checkbox-widget': {
    display: 'inline-block',
    width: '18px',
    height: '18px',
    marginRight: '8px',
    fontSize: '16px',
    color: '#64748b',
    cursor: 'pointer',
  },
  '.cm-checkbox-widget.checked': {
    color: '#22c55e',
  },
  '.cm-hr-widget': {
    display: 'block',
    border: 'none',
    borderTop: '1px solid rgba(148,163,184,0.2)',
    margin: '16px 0',
  },
  '.cm-mermaid-hidden-line': {
    height: '0 !important',
    padding: '0 !important',
    margin: '0 !important',
    overflow: 'hidden !important',
    opacity: '0',
    fontSize: '0',
    lineHeight: '0',
  },
  // YAML frontmatter lines — reset any styling the markdown extension or
  // livePreviewTheme would otherwise apply (the H1 styling on lines that
  // start with `#`, for example). Inside frontmatter, `#` is a YAML
  // comment, not a header. Use !important to win against the existing
  // header rules below without rearranging selector order.
  '.cm-frontmatter-line': {
    fontSize: '0.9em !important',
    fontWeight: 'normal !important',
    color: 'inherit !important',
    fontFamily:
      'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
    lineHeight: '1.4 !important',
  },
  '.cm-frontmatter-line *': {
    fontSize: 'inherit !important',
    fontWeight: 'normal !important',
    color: 'inherit !important',
    textDecoration: 'none !important',
    backgroundColor: 'transparent !important',
  },
  '.cm-codefence-hidden-line': {
    height: '0 !important',
    padding: '0 !important',
    margin: '0 !important',
    overflow: 'hidden !important',
    opacity: '0',
    fontSize: '0',
    lineHeight: '0',
  },
  '.cm-code-block-line': {
    backgroundColor: 'rgba(148,163,184,0.10)',
    paddingLeft: '12px !important',
    paddingRight: '12px !important',
    fontFamily:
      'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
    fontSize: '0.9em',
  },
  '.cm-code-block-line-first': {
    paddingTop: '8px !important',
    borderTopLeftRadius: '6px',
    borderTopRightRadius: '6px',
  },
  '.cm-code-block-line-last': {
    paddingBottom: '8px !important',
    borderBottomLeftRadius: '6px',
    borderBottomRightRadius: '6px',
  },

  // Rendered GFM table
  '.cm-table-hidden-line': {
    height: '0 !important',
    padding: '0 !important',
    margin: '0 !important',
    overflow: 'hidden !important',
    opacity: '0',
    fontSize: '0',
    lineHeight: '0',
  },
  '.cm-table-widget': {
    display: 'block',
    margin: '8px 0',
    overflowX: 'auto',
  },
  '.cm-rendered-table': {
    borderCollapse: 'collapse',
    width: '100%',
    fontSize: '0.92em',
    border: '1px solid rgba(148,163,184,0.3)',
  },
  '.cm-rendered-table th, .cm-rendered-table td': {
    border: '1px solid rgba(148,163,184,0.25)',
    padding: '6px 10px',
    textAlign: 'left',
    verticalAlign: 'top',
  },
  '.cm-rendered-table th': {
    backgroundColor: 'rgba(148,163,184,0.12)',
    fontWeight: '600',
  },
  '.cm-rendered-table tbody tr:nth-child(even) td': {
    backgroundColor: 'rgba(148,163,184,0.05)',
  },
});

// Light-mode overrides
export const livePreviewLightTheme = EditorView.theme({
  '.cm-header-line.cm-header-1': { color: '#1e293b' },
  '.cm-header-line.cm-header-2': { color: '#1e293b' },
  '.cm-header-line.cm-header-3': { color: '#1e293b' },
  '.cm-rendered-strong': { color: '#1e293b' },
  '.cm-rendered-emphasis': { color: '#475569' },
  '.cm-rendered-strike': { color: '#94a3b8' },
  '.cm-rendered-link': { color: '#2563eb' },
  '.cm-rendered-code': {
    backgroundColor: 'rgba(37,99,235,0.1)',
    color: '#2563eb',
  },
  '.cm-blockquote-line': { borderLeft: '3px solid #2563eb', color: '#94a3b8' },
  '.cm-list-bullet': { color: '#2563eb' },
});
