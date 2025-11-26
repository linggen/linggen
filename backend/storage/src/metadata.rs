use anyhow::Result;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use rememberme_core::{IndexingJob, SourceConfig};
use std::path::Path;
use std::sync::Arc;

use crate::preferences::UserPreferences;
use crate::profile::SourceProfile;

const SETTINGS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("settings");
const SOURCES_TABLE: TableDefinition<&str, &str> = TableDefinition::new("sources");
const JOBS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("jobs");
const PREFERENCES_TABLE: TableDefinition<&str, &str> = TableDefinition::new("preferences");
const PROFILE_TABLE: TableDefinition<&str, &str> = TableDefinition::new("profile");

pub struct MetadataStore {
    db: Arc<Database>,
}

impl MetadataStore {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let db = Database::create(path)?;

        // Initialize tables on first creation
        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(SETTINGS_TABLE)?;
            let _ = write_txn.open_table(SOURCES_TABLE)?;
            let _ = write_txn.open_table(JOBS_TABLE)?;
            let _ = write_txn.open_table(PREFERENCES_TABLE)?;
            let _ = write_txn.open_table(PROFILE_TABLE)?;
        }
        write_txn.commit()?;

        Ok(Self { db: Arc::new(db) })
    }

    // Settings
    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(SETTINGS_TABLE)?;
        let value = table.get(key)?.map(|v| v.value().to_string());
        Ok(value)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(SETTINGS_TABLE)?;
            table.insert(key, value)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    // Sources
    pub fn add_source(&self, source: &SourceConfig) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(SOURCES_TABLE)?;
            let value = serde_json::to_string(source)?;
            table.insert(source.id.as_str(), value.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_sources(&self) -> Result<Vec<SourceConfig>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(SOURCES_TABLE)?;
        let mut sources = Vec::new();
        for result in table.iter()? {
            let (_, value) = result?;
            let source: SourceConfig = serde_json::from_str(value.value())?;
            sources.push(source);
        }
        Ok(sources)
    }

    pub fn remove_source(&self, id: &str) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(SOURCES_TABLE)?;
            table.remove(id)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn get_source(&self, id: &str) -> Result<Option<SourceConfig>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(SOURCES_TABLE)?;
        if let Some(value) = table.get(id)? {
            let source: SourceConfig = serde_json::from_str(value.value())?;
            Ok(Some(source))
        } else {
            Ok(None)
        }
    }

    pub fn update_source(&self, source: &SourceConfig) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(SOURCES_TABLE)?;
            let value = serde_json::to_string(source)?;
            table.insert(source.id.as_str(), value.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    // Jobs
    pub fn create_job(&self, job: &IndexingJob) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(JOBS_TABLE)?;
            let json = serde_json::to_string(job)?;
            table.insert(job.id.as_str(), json.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    pub fn update_job(&self, job: &IndexingJob) -> Result<()> {
        self.create_job(job) // Same as create, just overwrites
    }

    pub fn get_job(&self, id: &str) -> Result<Option<IndexingJob>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(JOBS_TABLE)?;

        if let Some(value) = table.get(id)? {
            let json = value.value();
            let job: IndexingJob = serde_json::from_str(json)?;
            Ok(Some(job))
        } else {
            Ok(None)
        }
    }

    pub fn get_jobs(&self, limit: Option<usize>) -> Result<Vec<IndexingJob>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(JOBS_TABLE)?;

        let mut jobs = Vec::new();
        for item in table.iter()? {
            let (_, value) = item?;
            let json = value.value();
            let job: IndexingJob = serde_json::from_str(json)?;
            jobs.push(job);
        }

        // Sort by started_at descending (newest first)
        jobs.sort_by(|a, b| b.started_at.cmp(&a.started_at));

        if let Some(limit) = limit {
            jobs.truncate(limit);
        }

        Ok(jobs)
    }

    pub fn get_latest_job_for_source(&self, source_id: &str) -> Result<Option<IndexingJob>> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(JOBS_TABLE)?;

        let mut latest_job: Option<IndexingJob> = None;

        for item in table.iter()? {
            let (_, value) = item?;
            let json = value.value();
            let job: IndexingJob = serde_json::from_str(json)?;

            if job.source_id == source_id {
                match &latest_job {
                    None => latest_job = Some(job),
                    Some(current_latest) => {
                        if job.started_at > current_latest.started_at {
                            latest_job = Some(job);
                        }
                    }
                }
            }
        }

        Ok(latest_job)
    }

    // User Preferences
    /// Get user preferences (returns defaults if not set)
    pub fn get_preferences(&self) -> Result<UserPreferences> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(PREFERENCES_TABLE)?;

        if let Some(prefs_json) = table.get("user_prefs")? {
            let prefs: UserPreferences = serde_json::from_str(prefs_json.value())?;
            Ok(prefs)
        } else {
            // Return defaults if not set
            Ok(UserPreferences::default())
        }
    }

    /// Update user preferences
    pub fn update_preferences(&self, prefs: &UserPreferences) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(PREFERENCES_TABLE)?;
            let json = serde_json::to_string(prefs)?;
            table.insert("user_prefs", json.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    // Source Profiles
    /// Get source profile (returns default if not set)
    pub fn get_source_profile(&self, source_id: &str) -> Result<SourceProfile> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(PROFILE_TABLE)?;

        if let Some(json) = table.get(source_id)? {
            let profile: SourceProfile = serde_json::from_str(json.value())?;
            Ok(profile)
        } else {
            Ok(SourceProfile::default())
        }
    }

    /// Update source profile
    pub fn update_source_profile(&self, source_id: &str, profile: &SourceProfile) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(PROFILE_TABLE)?;
            let json = serde_json::to_string(profile)?;
            table.insert(source_id, json.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rememberme_core::SourceType;
    use tempfile::NamedTempFile;

    #[test]
    fn test_settings() -> Result<()> {
        let tmp_file = NamedTempFile::new()?;
        let store = MetadataStore::new(tmp_file.path())?;

        store.set_setting("theme", "dark")?;
        assert_eq!(store.get_setting("theme")?, Some("dark".to_string()));
        assert_eq!(store.get_setting("font")?, None);

        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use rememberme_core::{IndexingJob, JobStatus};
        use tempfile::NamedTempFile;

        #[test]
        fn test_sources() -> Result<()> {
            let tmp_file = NamedTempFile::new()?;
            let store = MetadataStore::new(tmp_file.path())?;

            let source = SourceConfig {
                id: "1".to_string(),
                name: "My Docs".to_string(),
                source_type: SourceType::Local,
                path: "/tmp/docs".to_string(),
                enabled: true,
                chunk_count: None,
                file_count: None,
                total_size_bytes: None,
            };

            store.add_source(&source)?;
            let sources = store.get_sources()?;
            assert_eq!(sources.len(), 1);
            assert_eq!(sources[0].name, "My Docs");

            store.remove_source("1")?;
            let sources = store.get_sources()?;
            assert_eq!(sources.len(), 0);

            Ok(())
        }

        #[test]
        fn test_latest_job() -> Result<()> {
            let tmp_file = NamedTempFile::new()?;
            let store = MetadataStore::new(tmp_file.path())?;

            let job1 = IndexingJob {
                id: "j1".to_string(),
                source_id: "s1".to_string(),
                source_name: "Source 1".to_string(),
                source_type: SourceType::Local,
                status: JobStatus::Completed,
                started_at: "2023-01-01T10:00:00Z".to_string(),
                finished_at: None,
                files_indexed: None,
                chunks_created: None,
                total_files: None,
                total_size_bytes: None,
                error: None,
            };

            let job2 = IndexingJob {
                id: "j2".to_string(),
                source_id: "s1".to_string(),
                source_name: "Source 1".to_string(),
                source_type: SourceType::Local,
                status: JobStatus::Completed,
                started_at: "2023-01-02T10:00:00Z".to_string(), // Later
                finished_at: None,
                files_indexed: None,
                chunks_created: None,
                total_files: None,
                total_size_bytes: None,
                error: None,
            };

            let job3 = IndexingJob {
                id: "j3".to_string(),
                source_id: "s2".to_string(), // Different source
                source_name: "Source 2".to_string(),
                source_type: SourceType::Local,
                status: JobStatus::Completed,
                started_at: "2023-01-03T10:00:00Z".to_string(),
                finished_at: None,
                files_indexed: None,
                chunks_created: None,
                total_files: None,
                total_size_bytes: None,
                error: None,
            };

            store.create_job(&job1)?;
            store.create_job(&job2)?;
            store.create_job(&job3)?;

            let latest_s1 = store.get_latest_job_for_source("s1")?;
            assert!(latest_s1.is_some());
            assert_eq!(latest_s1.unwrap().id, "j2");

            let latest_s2 = store.get_latest_job_for_source("s2")?;
            assert!(latest_s2.is_some());
            assert_eq!(latest_s2.unwrap().id, "j3");

            let latest_s3 = store.get_latest_job_for_source("s3")?;
            assert!(latest_s3.is_none());

            Ok(())
        }
    }
}
