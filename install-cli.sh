#!/bin/bash
set -e

echo "🔨 Building Linggen CLI..."
cd "$(dirname "$0")/backend"
cargo build --release --bin linggen

echo ""
echo "✅ Build complete!"
echo ""
echo "Binary location: $(pwd)/target/release/linggen"
echo ""
echo "To install globally, run:"
echo "  sudo cp target/release/linggen /usr/local/bin/"
echo ""
echo "Or add to your PATH by adding this to your ~/.bashrc or ~/.zshrc:"
echo "  export PATH=\"$(pwd)/target/release:\$PATH\""
echo ""
echo "Test the installation with:"
echo "  linggen --help"
