# Dockerfile for building Linggen Linux .deb package
FROM rust:1.83-bookworm

# Install system dependencies for Tauri and the backend
RUN apt-get update && apt-get install -y \
    # Tauri/GTK dependencies
    libwebkit2gtk-4.1-dev \
    libappindicator3-dev \
    librsvg2-dev \
    patchelf \
    libssl-dev \
    libgtk-3-dev \
    libayatana-appindicator3-dev \
    # For PDF extraction (poppler)
    libpoppler-glib-dev \
    # For .deb packaging
    dpkg \
    # Build tools
    pkg-config \
    cmake \
    && rm -rf /var/lib/apt/lists/*

# Install Node.js 20
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y nodejs

# Install Tauri CLI
RUN cargo install tauri-cli

WORKDIR /app

# Copy the project
COPY . .

# Build the backend binary first
WORKDIR /app/backend
RUN cargo build --release -p api

# Copy backend binary to where Tauri expects it
WORKDIR /app/frontend/src-tauri
RUN mkdir -p binaries && \
    cp /app/backend/target/release/api binaries/linggen-backend-x86_64-unknown-linux-gnu

# Install frontend dependencies
WORKDIR /app/frontend
RUN npm ci

# Build the Tauri app for Linux (.deb and .AppImage)
RUN cargo tauri build --bundles deb,appimage

# The .deb file will be in /app/frontend/src-tauri/target/release/bundle/deb/
