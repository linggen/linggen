# Linggen Analytics Worker

A Cloudflare Worker that collects anonymous usage analytics from the Linggen desktop app.

## What's Tracked

We collect minimal, anonymous data to understand how Linggen is being used:

- **app_started**: When the app launches (to measure DAU/MAU)
- **source_added**: When a user adds a project source (to understand usage patterns)

### Data Collected Per Event

- `installation_id`: Anonymous UUID per device (no personal info)
- `event_type`: Type of event
- `app_version`: App version for compatibility tracking
- `platform`: OS (macos/windows/linux)
- `payload`: Event-specific data (e.g., source type, project size bucket)

**No code content, file paths, or personal information is ever collected.**

## Setup

### 1. Install Dependencies

```bash
cd cf-worker
npm install
```

### 2. Create D1 Database

```bash
# Create the database
npx wrangler d1 create linggen_analytics

# Copy the database_id from the output and update wrangler.toml
```

### 3. Run Migrations

```bash
# Local development
npm run db:migrate

# Production
npm run db:migrate:prod
```

### 4. Set API_KEY (Required)

This worker requires an API key for **all** endpoints (analytics + skill registry).

```bash
npx wrangler secret put API_KEY
# Enter your secret key when prompted
```

### 5. Security Note

The `linggen-cli` can have an `API_KEY` baked in at compile time using the `LINGGEN_BUILD_API_KEY` environment variable. This allows the CLI to work out-of-the-box for users while still providing basic protection against unauthorized access.

To prevent abuse, the worker implements:
- **IP-based Rate Limiting**: Maximum 100 requests per hour per IP.
- **Privacy-focused Hashing**: IP addresses are salted and hashed before storage.


## Development

```bash
# Start local dev server
npm run dev
```

Test with curl:

```bash
curl -X POST http://localhost:8787/track \
  -H "X-API-Key: $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "installation_id": "test-uuid-1234",
    "event_type": "app_started",
    "app_version": "0.5.0",
    "platform": "macos"
  }'
```

## Deployment

```bash
npm run deploy
```

## API Endpoints

### POST /track

Record an analytics event.

**Request Body:**

```json
{
  "installation_id": "uuid-string",
  "event_type": "app_started" | "source_added",
  "app_version": "0.5.0",
  "platform": "macos" | "windows" | "linux" | "unknown",
  "payload": {
    // For source_added:
    "source_type": "local" | "git" | "web" | "uploads",
    "size_bucket": "small" | "medium" | "large" | "xlarge",
    "file_count": 150
  }
}
```

**Response:**

```json
{
  "success": true,
  "event_id": "123"
}
```

### GET /skills

List all recorded skills (public). Supports pagination.

**Query Parameters:**
- `page`: Page number (default: 1)
- `limit`: Items per page (default: 20, max: 100)

**Response:**

```json
{
  "success": true,
  "skills": [
    {
      "skill_id": "https://github.com/owner/repo/skill-name",
      "url": "https://github.com/owner/repo",
      "skill": "skill-name",
      "ref": "main",
      "content": "...",
      "install_count": 5,
      "updated_at": "2024-01-15T10:30:00.000Z"
    }
  ],
  "pagination": {
    "total": 1,
    "page": 1,
    "limit": 20,
    "total_pages": 1
  }
}
```

### GET /health

Health check endpoint.

**Response:**

```json
{
  "status": "ok",
  "timestamp": "2024-01-15T10:30:00.000Z"
}
```

## Querying Data

See [QUERIES.md](./QUERIES.md) for useful SQL queries to analyze the collected data.
# Query events in D1
npx wrangler d1 execute linggen_analytics --remote --command "SELECT * FROM events"
