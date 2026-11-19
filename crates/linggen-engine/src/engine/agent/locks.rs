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

    /// An expired lock is no lock, whether or not `acquire` has swept it yet.
    pub fn is_locked_by_other(&self, agent_id: &str, path: &str) -> bool {
        let now = Instant::now();
        for (glob_str, info) in &self.locks {
            if info.owner_id != agent_id && info.expires_at > now {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_expired_lock_refuses_no_one() {
        let mut locks = LockManager::new();
        let got = locks.acquire("a", vec!["src/**".into()], Duration::from_secs(60));
        assert_eq!(got.acquired.len(), 1);
        assert!(locks.is_locked_by_other("b", "src/x.rs"), "a live lock refuses others");
        assert!(!locks.is_locked_by_other("a", "src/x.rs"), "never its owner");
        locks.locks.get_mut("src/**").unwrap().expires_at = Instant::now() - Duration::from_secs(1);
        assert!(!locks.is_locked_by_other("b", "src/x.rs"), "an expired lock is no lock");
    }
}
