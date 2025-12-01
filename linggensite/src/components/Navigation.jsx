import { useState, useEffect } from 'react'
import logo from '../assets/logo.svg'

function Navigation() {
    const [scrolled, setScrolled] = useState(false)
    const [mobileMenuOpen, setMobileMenuOpen] = useState(false)

    useEffect(() => {
        const handleScroll = () => {
            setScrolled(window.scrollY > 50)
        }

        window.addEventListener('scroll', handleScroll)
        return () => window.removeEventListener('scroll', handleScroll)
    }, [])

    const scrollToSection = (id) => {
        const element = document.getElementById(id)
        if (element) {
            element.scrollIntoView({ behavior: 'smooth' })
            setMobileMenuOpen(false)
        }
    }

    return (
        <nav className={`navigation ${scrolled ? 'scrolled' : ''}`}>
            <div className="nav-container">
                <div className="nav-brand" onClick={() => scrollToSection('home')}>
                    <img src={logo} alt="Linggen Logo" className="nav-logo" />
                    <span className="nav-title">Linggen</span>
                </div>

                <button
                    className="mobile-menu-toggle"
                    onClick={() => setMobileMenuOpen(!mobileMenuOpen)}
                    aria-label="Toggle menu"
                >
                    {mobileMenuOpen ? '✕' : '☰'}
                </button>

                <div className={`nav-links ${mobileMenuOpen ? 'mobile-open' : ''}`}>
                    <a onClick={() => scrollToSection('features')}>Features</a>
                    <a onClick={() => scrollToSection('demo')}>Demo</a>
                    <a onClick={() => scrollToSection('get-started')}>Get Started</a>
                </div>
            </div>
        </nav>
    )
}

export default Navigation
