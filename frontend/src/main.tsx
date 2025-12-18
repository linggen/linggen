import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import ExtensionApp from './ExtensionApp.tsx'

const isExtension = window.location.pathname.startsWith('/extension')

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    {isExtension ? <ExtensionApp /> : <App />}
  </StrictMode>,
)
