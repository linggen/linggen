function BetaDisclaimer() {
    return (
        <section className="beta-section" id="beta">
            <div className="container">
                <div className="beta-card">
                    <h3>🚀 Public Beta</h3>
                    <p>
                        Linggen is currently in <strong>public beta</strong> for macOS. 
                        Windows and Linux support is on the roadmap.
                    </p>
                    <ul>
                        <li>All data stays on your device – nothing is sent to external servers</li>
                        <li>Embeddings and LLM models are stored locally after initial download</li>
                        <li>We'd love your feedback to make Linggen better</li>
                    </ul>
                </div>
            </div>
        </section>
    )
}

export default BetaDisclaimer
