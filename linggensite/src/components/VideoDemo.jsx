function VideoDemo() {
    return (
        <section className="demo-section" id="demo">
            <div className="container">
                <h2 className="section-title">
                    <span className="title-decoration">◆</span>
                    See It in Action
                    <span className="title-decoration">◆</span>
                </h2>
                <p className="section-description">
                    A short walkthrough of how Linggen helps you index and search your code and docs.
                </p>

                <div className="video-container">
                    <div className="video-placeholder">
                        <div className="video-icon">▶</div>
                        <p>Demo video coming soon</p>
                        <small>In the meantime, download the beta and try it yourself!</small>
                    </div>
                    {/* Uncomment and add your video URL when ready:
                    <iframe 
                        src="YOUR_YOUTUBE_OR_LOOM_URL" 
                        frameBorder="0" 
                        allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture" 
                        allowFullScreen>
                    </iframe>
                    */}
                </div>
            </div>
        </section>
    )
}

export default VideoDemo
