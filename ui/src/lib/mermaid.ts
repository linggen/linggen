// The one mermaid instance for the whole UI. Chat and the markdown editor
// used to each initialize it (one `strict`, one `loose`), and whichever ran
// last won for both; model-written diagrams must always render `strict`.
type Mermaid = typeof import('mermaid').default;

let instance: Promise<Mermaid | null> | null = null;

export function getMermaid(): Promise<Mermaid | null> {
  if (!instance) {
    instance = import('mermaid')
      .then(({ default: mermaid }) => {
        const isDark =
          document.documentElement.classList.contains('dark') ||
          !!window.matchMedia?.('(prefers-color-scheme: dark)').matches;
        mermaid.initialize({
          startOnLoad: false,
          securityLevel: 'strict',
          theme: isDark ? 'dark' : 'default',
        });
        return mermaid;
      })
      .catch((err) => {
        console.error('Failed to load mermaid:', err);
        instance = null;
        return null;
      });
  }
  return instance;
}
