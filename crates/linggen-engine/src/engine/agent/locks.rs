use globset::Glob;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct LockManager {
    // path_glob -> (owner_agent_id, expires_at, token)
    pub locks: HashMap<String, LockInfo>,
}

pub struct LockInfo {
    pub owner_id: String,
    pub expires_at: Instant,
    pub token: String,
}

impl LockManager {
    pub fn new() -> Self {
        Self {
            locks: HashMap::new(),
        }
    }

    pub fn acquire(&mut self, agent_id: &str, globs: Vec<String>, ttl: Duration) -> LockResult {
        let now = Instant::now();
        self.cleanup(now);

        let mut acquired = Vec::new();
        let mut denied = Vec::new();

        for glob_str in globs {
            if let Some(info) = self.locks.get(&glob_str) {
                if info.owner_id == agent_id {
                    // Re-entrant lock, update expiry
                    let token = info.token.clone();
                    self.locks.insert(
                        glob_str.clone(),
                        LockInfo {
                            owner_id: agent_id.to_string(),
                            expires_at: now + ttl,
                            token: token.clone(),
                        },
                    );
                    acquired.push((glob_str, token));
                } else {
                    denied.push(glob_str);
                }
            } else {
                let token = uuid::Uuid::new_v4().to_string();
                self.locks.insert(
                    glob_str.clone(),
                    LockInfo {
                        owner_id: agent_id.to_string(),
                        expires_at: now + ttl,
                        token: token.clone(),
                    },
                );
                acquired.push((glob_str, token));
            }
        }

        LockResult { acquired, denied }
    }

    pub fn release(&mut self, agent_id: &str, tokens: Vec<String>) {
        self.locks
            .retain(|_, info| !(info.owner_id == agent_id && tokens.contains(&info.token)));
    }

    pub fn is_locked_by_other(&self, agent_id: &str, path: &str) -> bool {
        for (glob_str, info) in &self.locks {
            if info.owner_id != agent_id {
                if let Ok(glob) = Glob::new(glob_str) {
                    if glob.compile_matcher().is_match(path) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn cleanup(&mut self, now: Instant) {
        self.locks.retain(|_, info| info.expires_at > now);
    }
}

pub struct LockResult {
    pub acquired: Vec<(String, String)>,
    pub denied: Vec<String>,
}
