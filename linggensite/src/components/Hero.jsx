import logo from '../assets/logo.svg'
import appScreenshot from '../assets/sources.png'
import { DOWNLOAD_URL } from '../constants'

function Hero() {
    return (
        <section className="hero" id="home">
            <div className="container">
                <img src={logo} alt="Linggen Logo" className="hero-logo" />
                <h1 className="hero-title">
                    <span className="brand-name">Linggen</span>
                </h1>
                <p className="hero-subtitle">Your local-first memory layer for vibe coding</p>
                <p className="hero-description">
                    <strong>Linggen</strong> indexes your projects, documents, and notes on your own machine,
                    then lets you search and chat with them using AI – with your data staying completely local.
                </p>
                <div className="cta-buttons">
                    <a href={DOWNLOAD_URL} className="btn btn-primary" target="_blank" rel="noopener noreferrer">
                        Download for macOS (Beta)
                    </a>
                    <a href="#features" className="btn btn-secondary">Explore Features</a>
                </div>
                <p className="hero-note">
                    Windows &amp; Linux support coming soon.
                </p>

                <div className="app-preview">
                    <img 
                        src={appScreenshot} 
                        alt="Linggen App - Sources View" 
                        className="app-screenshot"
                    />
                </div>
            </div>

            <div className="scroll-indicator">
                <div className="scroll-arrow"></div>
            </div>
        </section>
    )
}

export default Hero
