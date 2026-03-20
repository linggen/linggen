import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { ErrorBoundary } from './components/ErrorBoundary';
import './index.css';
import { installFetchProxy } from './lib/fetchProxy';

// Install transport-aware fetch proxy so all /api/* calls route through
// WebRTC when using RtcTransport (required for remote mode).
installFetchProxy();

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>
);
