#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

echo "🔨 Building Linggen for distribution..."

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Build frontend
echo -e "${BLUE}📦 Building frontend...${NC}"
cd frontend
npm ci
npm run build
cd ..

# Build backend
echo -e "${BLUE}🦀 Building backend (release mode)...${NC}"
cd backend
cargo build --release --package api
cd ..

# Create distribution directory (standalone server, not Tauri app)
PLATFORM=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)
DIST_DIR="dist/standalone-${PLATFORM}-${ARCH}"

echo -e "${BLUE}📁 Creating distribution package...${NC}"
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"
cp backend/target/release/api "$DIST_DIR/linggen"
cp -r frontend/dist "$DIST_DIR/frontend"
mkdir -p "$DIST_DIR/data"

# Create README for users
cat > "$DIST_DIR/README.txt" << 'EOF'
Linggen RAG - Local Semantic Search (Standalone Server)

To run:
  ./linggen

The application will start on http://localhost:7000
Open your browser and navigate to that URL.

Data will be stored in the ./data directory.

Requirements:
- Internet connection for first-time model download
- ~500MB disk space for models and data
EOF

# Create run script
cat > "$DIST_DIR/run.sh" << 'EOF'
#!/bin/bash
cd "$(dirname "$0")"
echo "Starting Linggen..."
echo "Open your browser to: http://localhost:7000"
./linggen
EOF
chmod +x "$DIST_DIR/run.sh"

echo -e "${GREEN}✅ Build complete!${NC}"
echo -e "Distribution package: ${BLUE}${DIST_DIR}/${NC}"
echo ""
echo "To test:"
echo "  cd $DIST_DIR"
echo "  ./run.sh"
echo ""
echo "To create archive:"
echo "  tar -czf dist/linggen-standalone-${PLATFORM}-${ARCH}.tar.gz -C dist standalone-${PLATFORM}-${ARCH}/"

