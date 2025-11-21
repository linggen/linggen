use anyhow::Result;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use rememberme_core::SourceConfig;
use std::path::Path;
use std::sync::Arc;

const SETTINGS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("settings");
const SOURCES_TABLE: TableDefinition<&str, &str> = TableDefinition::new("sources");

pub struct MetadataStore {
    db: Arc<Database>,
}

impl MetadataStore {
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let db = Database::create(path)?;
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
}
