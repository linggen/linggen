import logo from '../assets/logo.svg'

function Hero() {
    return (
        <section className="hero" id="home">
            <div className="container">
                <img src={logo} alt="Linggen Logo" className="hero-logo" />
                <h1 className="hero-title">
                    <span className="brand-name">Linggen</span>
                </h1>
                <p className="hero-subtitle">Local-first AI for your code and knowledge</p>
                <p className="hero-description">
                    <strong>Linggen</strong> indexes your projects, documents, and notes on your own machine,
                    then lets you search and chat with them using AI – with your data staying completely local.
                </p>
                <div className="cta-buttons">
                    <a href="#get-started" className="btn btn-primary">
                        Download for macOS (Beta)
                    </a>
                    <a href="#features" className="btn btn-secondary">Explore Features</a>
                </div>
                <p className="hero-note">
                    Windows &amp; Linux support coming soon.
                </p>
            </div>

            <div className="scroll-indicator">
                <div className="scroll-arrow"></div>
            </div>
        </section>
    )
}

export default Hero
