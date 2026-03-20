import React from 'react';

interface Props {
  children: React.ReactNode;
  fallback?: React.ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

/**
 * Top-level error boundary — catches render errors so a single component
 * crash doesn't white-out the entire page.
 */
export class ErrorBoundary extends React.Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error('ErrorBoundary caught:', error, info.componentStack);
  }

  render() {
    if (this.state.hasError) {
      if (this.props.fallback) return this.props.fallback;
      return (
        <div style={{ padding: 24, fontFamily: 'monospace' }}>
          <h2 style={{ color: '#e53e3e', margin: '0 0 12px' }}>Something went wrong</h2>
          <pre style={{ whiteSpace: 'pre-wrap', color: '#666', fontSize: 13 }}>
            {this.state.error?.message}
          </pre>
          <button
            onClick={() => this.setState({ hasError: false, error: null })}
            style={{ marginTop: 12, padding: '6px 16px', cursor: 'pointer' }}
          >
            Try again
          </button>
          <button
            onClick={() => window.location.reload()}
            style={{ marginTop: 12, marginLeft: 8, padding: '6px 16px', cursor: 'pointer' }}
          >
            Reload page
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
