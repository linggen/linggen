function VideoDemo() {
    return (
        <section className="demo-section" id="demo">
            <div className="container">
                <h2 className="section-title">
                    <span className="title-decoration">◆</span>
                    Witness the Power
                    <span className="title-decoration">◆</span>
                </h2>
                <p className="section-description">
                    See how Linggen transforms your knowledge management workflow with AI-enhanced memory and instant contextual recall.
                </p>

                <div className="video-container">
                    <div className="video-placeholder">
                        <div className="video-icon">▶</div>
                        <p>Add your introduction video here</p>
                        <small>Replace this section with your demo video</small>
                    </div>
                    {/* Uncomment and add your video URL:
          <iframe 
            src="YOUR_VIDEO_URL" 
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
