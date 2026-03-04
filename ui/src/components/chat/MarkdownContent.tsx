import React, { useEffect, useId, useRef, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeHighlight from 'rehype-highlight';
import { getMermaid, hashText, normalizeMarkdownish } from './utils/markdown';

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

export const MarkdownContent: React.FC<{ text: string }> = ({ text }) => (
  <div className="markdown-body break-words">
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      rehypePlugins={[rehypeHighlight]}
      components={{
        pre: ({ children }) => <>{children}</>,
        code: ({ inline, className, children, node: _node, ...props }: any) => {
          // Extract raw text from children (may be React elements from rehype-highlight).
          const extractText = (node: unknown): string => {
            if (typeof node === 'string') return node;
            if (Array.isArray(node)) return node.map(extractText).join('');
            if (node && typeof node === 'object' && 'props' in node) {
              return extractText((node as any).props?.children);
            }
            return '';
          };
          const rawText = extractText(children).replace(/\n$/, '');

          const match = /language-([\w-]+)/.exec(className || '');
          const lang = match?.[1]?.toLowerCase();
          if (!inline && lang === 'mermaid') {
            return <MermaidBlock code={rawText} />;
          }
          const isInlineCode = Boolean(inline) || (!className && !rawText.includes('\n'));
          if (isInlineCode) {
            return <code {...props}>{children}</code>;
          }
          return (
            <pre>
              <code className={className} {...props}>{children}</code>
            </pre>
          );
        },
      }}
    >
      {normalizeMarkdownish(text)}
    </ReactMarkdown>
  </div>
);
