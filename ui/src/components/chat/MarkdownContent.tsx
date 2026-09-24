import React, { useEffect, useId, useRef, useState } from 'react';
import ReactMarkdown, { type Components } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeHighlight from 'rehype-highlight';
import { hashText, normalizeMarkdownish } from './utils/markdown';
import { getMermaid } from '../../lib/mermaid';

const MermaidBlock: React.FC<{ code: string }> = ({ code }) => {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [error, setError] = useState<string | null>(null);
  const uniqueId = useId().replace(/:/g, '');
  const idRef = useRef(`chat-mermaid-${hashText(code)}-${uniqueId}`);

  useEffect(() => {
    let cancelled = false;

    const render = async () => {
      setError(null);
      if (!containerRef.current) return;
      containerRef.current.innerHTML = '<div class="markdown-mermaid-loading">Rendering Mermaid...</div>';
      try {
        const mermaid = await getMermaid();
        if (!mermaid) throw new Error('Mermaid not available');
        const { svg } = await mermaid.render(idRef.current, code.trim());
        if (!cancelled && containerRef.current) {
          containerRef.current.innerHTML = svg;
        }
      } catch (e) {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : String(e));
        }
      }
    };

    render();
    return () => {
      cancelled = true;
    };
  }, [code]);

  if (error) {
    return (
      <div className="markdown-mermaid-error">
        Mermaid error: {error}
      </div>
    );
  }
  return <div className="markdown-mermaid" ref={containerRef} />;
};

// Module-level so every render hands ReactMarkdown the same plugins and
// renderers; fresh ones each render would defeat the memo below.
const REMARK_PLUGINS = [remarkGfm];
const REHYPE_PLUGINS = [rehypeHighlight];
const MD_COMPONENTS: Components = {
  a: ({ href, children, node: _node, ...props }) => (
    <a href={href} target="_blank" rel="noopener noreferrer" {...props}>{children}</a>
  ),
  pre: ({ children }) => <>{children}</>,
  code: ({ className, children, node: _node, ...props }) => {
    // Extract raw text from children (may be React elements from rehype-highlight).
    const extractText = (node: unknown): string => {
      if (typeof node === 'string') return node;
      if (Array.isArray(node)) return node.map(extractText).join('');
      if (node && typeof node === 'object' && 'props' in node) {
        return extractText((node as { props?: { children?: unknown } }).props?.children);
      }
      return '';
    };
    const rawText = extractText(children).replace(/\n$/, '');

    const match = /language-([\w-]+)/.exec(className || '');
    const lang = match?.[1]?.toLowerCase();
    if (lang === 'mermaid') {
      return <MermaidBlock code={rawText} />;
    }
    // react-markdown 9+ passes no `inline`: a span is code with no language
    // class and no newline.
    const isInlineCode = !className && !rawText.includes('\n');
    if (isInlineCode) {
      return <code {...props}>{children}</code>;
    }
    return (
      <pre>
        <code className={className} {...props}>{children}</code>
      </pre>
    );
  },
};

/** Memoized on `text`: a finished message's markdown parses once, not once
 *  per streamed token of the message below it. */
export const MarkdownContent = React.memo<{ text: string }>(({ text }) => (
  <div className="markdown-body break-words">
    <ReactMarkdown remarkPlugins={REMARK_PLUGINS} rehypePlugins={REHYPE_PLUGINS} components={MD_COMPONENTS}>
      {normalizeMarkdownish(text.replace(/<!--[\s\S]*?-->/g, ''))}
    </ReactMarkdown>
  </div>
));
