#!/bin/bash
set -e

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

# Create distribution directory
echo -e "${BLUE}📁 Creating distribution package...${NC}"
mkdir -p dist/linggen
cp backend/target/release/api dist/linggen/linggen
cp -r frontend/dist dist/linggen/frontend
mkdir -p dist/linggen/data

# Create README for users
cat > dist/linggen/README.txt << 'EOF'
Linggen RAG - Local Semantic Search

To run:
  ./linggen

The application will start on http://localhost:3000
Open your browser and navigate to that URL.

Data will be stored in the ./data directory.

Requirements:
- Internet connection for first-time model download
- ~500MB disk space for models and data
EOF

# Create run script
cat > dist/linggen/run.sh << 'EOF'
#!/bin/bash
cd "$(dirname "$0")"
echo "Starting Linggen..."
echo "Open your browser to: http://localhost:3000"
./linggen
EOF
chmod +x dist/linggen/run.sh

echo -e "${GREEN}✅ Build complete!${NC}"
echo -e "Distribution package: ${BLUE}dist/linggen/${NC}"
echo ""
echo "To test:"
echo "  cd dist/linggen"
echo "  ./run.sh"
echo ""
echo "To create archive:"
echo "  cd dist"
echo "  tar -czf linggen-$(uname -s)-$(uname -m).tar.gz linggen/"

