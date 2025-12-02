function Footer() {
    return (
        <footer className="footer">
            <div className="container">
                <p className="footer-text">
                    <strong>Linggen</strong> – Local-first memory layer for your code and knowledge
                </p>
                <p className="footer-links">
                    <a href="mailto:hello@linggen.dev">Contact</a>
                    <span className="separator">·</span>
                    <a href="#get-started">Get Started</a>
                </p>
                <p className="footer-credit">
                    Built with Rust &amp; Tauri
                </p>
            </div>
        </footer>
    )
}

export default Footer
