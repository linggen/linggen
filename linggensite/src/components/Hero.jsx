import logo from '../assets/logo.png'

function Hero() {
    return (
        <section className="hero" id="home">
            <div className="container">
                <img src={logo} alt="Linggen Logo" className="hero-logo" />
                <h1 className="hero-title">
                    <span className="brand-name">Linggen</span>
                </h1>
                <p className="hero-subtitle">Cultivate Your Knowledge Spiritual Roots</p>
                <p className="hero-description">
                    Like spiritual roots that determine cultivation potential,
                    <strong> Linggen</strong> enhances your ability to absorb, store, and recall knowledge through AI-powered semantic search.
                </p>
                <div className="cta-buttons">
                    <a href="#get-started" className="btn btn-primary">Begin Your Journey</a>
                    <a href="#features" className="btn btn-secondary">Discover Features</a>
                </div>
            </div>

            <div className="scroll-indicator">
                <div className="scroll-arrow"></div>
            </div>
        </section>
    )
}

export default Hero
