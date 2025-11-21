#!/bin/bash
set -e

echo "🔨 Building RememberMe for distribution..."

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
mkdir -p dist/rememberme
cp backend/target/release/api dist/rememberme/rememberme
cp -r frontend/dist dist/rememberme/frontend
mkdir -p dist/rememberme/data

# Create README for users
cat > dist/rememberme/README.txt << 'EOF'
RememberMe RAG - Local Semantic Search

To run:
  ./rememberme

The application will start on http://localhost:3000
Open your browser and navigate to that URL.

Data will be stored in the ./data directory.

Requirements:
- Internet connection for first-time model download
- ~500MB disk space for models and data
EOF

# Create run script
cat > dist/rememberme/run.sh << 'EOF'
#!/bin/bash
cd "$(dirname "$0")"
echo "Starting RememberMe..."
echo "Open your browser to: http://localhost:3000"
./rememberme
EOF
chmod +x dist/rememberme/run.sh

echo -e "${GREEN}✅ Build complete!${NC}"
echo -e "Distribution package: ${BLUE}dist/rememberme/${NC}"
echo ""
echo "To test:"
echo "  cd dist/rememberme"
echo "  ./run.sh"
echo ""
echo "To create archive:"
echo "  cd dist"
echo "  tar -czf rememberme-$(uname -s)-$(uname -m).tar.gz rememberme/"

