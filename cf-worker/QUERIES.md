# Analytics Queries

Useful SQL queries for analyzing Linggen usage data.

## Basic Metrics

### Daily Active Users (DAU)

```sql
SELECT 
  DATE(created_at) as day,
  COUNT(DISTINCT installation_id) as unique_users
FROM events
WHERE event_type = 'app_started'
GROUP BY DATE(created_at)
ORDER BY day DESC
LIMIT 30;
```

### Weekly Active Users (WAU)

```sql
SELECT 
  strftime('%Y-W%W', created_at) as week,
  COUNT(DISTINCT installation_id) as unique_users
FROM events
WHERE event_type = 'app_started'
GROUP BY week
ORDER BY week DESC
LIMIT 12;
```

### Total Unique Installations

```sql
SELECT COUNT(DISTINCT installation_id) as total_installations
FROM events;
```

## Activation Metrics

### Daily New Sources Added

```sql
SELECT 
  DATE(created_at) as day,
  COUNT(*) as sources_added
FROM events
WHERE event_type = 'source_added'
GROUP BY DATE(created_at)
ORDER BY day DESC
LIMIT 30;
```

### Sources by Type

```sql
SELECT 
  json_extract(payload, '$.source_type') as source_type,
  COUNT(*) as count
FROM events
WHERE event_type = 'source_added'
GROUP BY source_type
ORDER BY count DESC;
```

### Project Size Distribution

```sql
SELECT 
  json_extract(payload, '$.size_bucket') as size_bucket,
  COUNT(*) as count
FROM events
WHERE event_type = 'source_added'
  AND payload IS NOT NULL
GROUP BY size_bucket
ORDER BY count DESC;
```

## Platform Distribution

### Users by Platform

```sql
SELECT 
  platform,
  COUNT(DISTINCT installation_id) as unique_users
FROM events
GROUP BY platform
ORDER BY unique_users DESC;
```

### Events by Platform (Last 7 Days)

```sql
SELECT 
  platform,
  event_type,
  COUNT(*) as count
FROM events
WHERE created_at >= datetime('now', '-7 days')
GROUP BY platform, event_type
ORDER BY platform, count DESC;
```

## Version Tracking

### Active Versions

```sql
SELECT 
  app_version,
  COUNT(DISTINCT installation_id) as unique_users,
  COUNT(*) as total_events
FROM events
WHERE created_at >= datetime('now', '-7 days')
GROUP BY app_version
ORDER BY unique_users DESC;
```

## Retention Analysis

### Users Who Added at Least One Source

```sql
SELECT 
  COUNT(DISTINCT e1.installation_id) as users_with_sources,
  (SELECT COUNT(DISTINCT installation_id) FROM events) as total_users
FROM events e1
WHERE e1.event_type = 'source_added';
```

### First Event Date Per User

```sql
SELECT 
  installation_id,
  MIN(created_at) as first_seen,
  MAX(created_at) as last_seen,
  COUNT(*) as total_events
FROM events
GROUP BY installation_id
ORDER BY first_seen DESC
LIMIT 100;
```

## Running Queries

### Via Wrangler CLI

```bash
# Local database
npx wrangler d1 execute linggen_analytics --local --command "SELECT COUNT(*) FROM events"

# Production database
npx wrangler d1 execute linggen_analytics --remote --command "SELECT COUNT(*) FROM events"
```

### Via Cloudflare Dashboard

1. Go to Cloudflare Dashboard → D1
2. Select `linggen_analytics` database
3. Use the SQL console to run queries
