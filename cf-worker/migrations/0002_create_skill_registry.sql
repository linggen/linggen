-- Create skill registry tables
CREATE TABLE IF NOT EXISTS skills (
    skill_id TEXT PRIMARY KEY, -- e.g. "https://github.com/anthropics/skills/frontend-design"
    url TEXT NOT NULL,
    skill TEXT NOT NULL,
    ref TEXT NOT NULL,
    content TEXT, -- The content of SKILL.md for preview
    install_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS skill_installs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    skill_id TEXT NOT NULL,
    ip_hash TEXT NOT NULL,
    bucket INTEGER NOT NULL, -- floor(unixepoch / 3600) for 1h cooldown
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(skill_id, ip_hash, bucket)
);

CREATE INDEX IF NOT EXISTS idx_skill_installs_skill_id ON skill_installs(skill_id);
CREATE INDEX IF NOT EXISTS idx_skill_installs_ip_hash ON skill_installs(ip_hash);
