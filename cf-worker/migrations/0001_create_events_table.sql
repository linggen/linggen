-- Create events table for Linggen analytics
-- This table stores anonymous usage events from the desktop app

CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    installation_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    app_version TEXT NOT NULL,
    platform TEXT NOT NULL,
    payload TEXT,  -- JSON string for event-specific data
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Index for querying by installation_id (useful for user journey analysis)
CREATE INDEX IF NOT EXISTS idx_events_installation_id ON events(installation_id);

-- Index for querying by event_type (useful for aggregations)
CREATE INDEX IF NOT EXISTS idx_events_event_type ON events(event_type);

-- Index for time-based queries (daily/weekly stats)
CREATE INDEX IF NOT EXISTS idx_events_created_at ON events(created_at);

-- Composite index for common query pattern: events by type and time
CREATE INDEX IF NOT EXISTS idx_events_type_time ON events(event_type, created_at);
