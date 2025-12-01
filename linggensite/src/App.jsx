import { useState, useEffect } from 'react'
import Navigation from './components/Navigation'
import Hero from './components/Hero'
import VideoDemo from './components/VideoDemo'
import Features from './components/Features'
import Documentation from './components/Documentation'
import GettingStarted from './components/GettingStarted'
import BetaDisclaimer from './components/BetaDisclaimer'
import Footer from './components/Footer'
import './App.css'

function App() {
  useEffect(() => {
    // Scroll-triggered animations
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

    // Observe all animatable elements
    document.querySelectorAll('.feature-card, .doc-card, .step, .video-container').forEach(el => {
      observer.observe(el)
    })

    return () => observer.disconnect()
  }, [])

  return (
    <div className="App">
      <div className="spiritual-energy"></div>

      <Navigation />
      <Hero />
      <VideoDemo />
      <Features />
      <Documentation />
      <GettingStarted />
      <BetaDisclaimer />
      <Footer />
    </div>
  )
}

export default App
