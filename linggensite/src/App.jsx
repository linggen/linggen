import { useEffect } from 'react'
import { BrowserRouter, Routes, Route, useLocation } from 'react-router-dom'
import HomePage from './pages/HomePage'
import DocsLayout from './pages/DocsLayout'
import QuickStart from './pages/docs/QuickStart'
import Sources from './pages/docs/Sources'
import MCPSetup from './pages/docs/MCPSetup'
import Search from './pages/docs/Search'
import './App.css'

function AppContent() {
  const location = useLocation()

  useEffect(() => {
    // Scroll-triggered animations - run after route change
    const observerOptions = {
      threshold: 0.1,
      rootMargin: '0px 0px -50px 0px'
    }

    const observer = new IntersectionObserver((entries) => {
      entries.forEach(entry => {
        if (entry.isIntersecting) {
          entry.target.classList.add('animate-in')
        }
      })
    }, observerOptions)

    // Small delay to ensure DOM is ready after route change
    const timeoutId = setTimeout(() => {
      // Observe all animatable elements
      document.querySelectorAll('.feature-card, .guide-card').forEach(el => {
        observer.observe(el)
      })
    }, 100)

    return () => {
      clearTimeout(timeoutId)
      observer.disconnect()
    }
  }, [location.pathname])

  return (
    <div className="App">
      <div className="spiritual-energy"></div>
      <Routes>
        <Route path="/" element={<HomePage />} />
        <Route path="/docs" element={<DocsLayout />}>
          <Route index element={<QuickStart />} />
          <Route path="sources" element={<Sources />} />
          <Route path="mcp" element={<MCPSetup />} />
          <Route path="search" element={<Search />} />
        </Route>
      </Routes>
    </div>
  )
}

function App() {
  return (
    <BrowserRouter>
      <AppContent />
    </BrowserRouter>
  )
}

export default App
