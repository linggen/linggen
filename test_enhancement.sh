#!/bin/bash

# RememberMe Prompt Enhancement System - Test Script
# This demonstrates the full 5-stage pipeline

BASE_URL="http://localhost:3000"

echo "🚀 Testing RememberMe Prompt Enhancement Pipeline"
echo "=================================================="
echo ""

# Test 1: Set User Preferences
echo "📝 Step 1: Setting user preferences..."
curl -s -X PUT "$BASE_URL/api/preferences" \
  -H "Content-Type: application/json" \
  -d '{
    "preferences": {
      "explanation_style": "concise",
      "output_format": "code_diff",
      "preferred_language": "Rust",
      "include_examples": false,
      "show_related_code": true,
      "max_explanation_words": 200
    }
  }' | echo "✅ Preferences set"

echo ""

# Test 2: Classify Intent Only
echo "🎯 Step 2: Testing intent classification..."
echo "Query: 'explain how the vector store works'"
curl -s -X POST "$BASE_URL/api/classify" \
  -H "Content-Type: application/json" \
  -d '{"query": "explain how the vector store works"}' | jq '.'

echo ""

# Test 3: Full Enhancement Pipeline  
echo "✨ Step 3: Testing FULL enhancement pipeline..."
echo "Query: 'fix the authentication timeout bug'"
echo ""
curl -s -X POST "$BASE_URL/api/enhance" \
  -H "Content-Type: application/json" \
  -d '{"query": "fix the authentication timeout bug"}' | jq '.'

echo ""
echo "=================================================="
echo "🎉 Test complete!"
echo ""
echo "Note: The first run will download the model (~2GB)"
echo "This may take 30-60 seconds. Subsequent runs are fast!"
