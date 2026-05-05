use std::path::{Path, PathBuf};

use async_trait::async_trait;
use runtime_core::{RuntimeError, RuntimeStore};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqliteStoreConfig {
    pub database_path: PathBuf,
}

#[derive(Debug)]
pub struct SqliteRuntimeStore {
    config: SqliteStoreConfig,
}

impl SqliteRuntimeStore {
    pub fn new(config: SqliteStoreConfig) -> Self {
        Self { config }
    }

    pub fn database_path(&self) -> &Path {
        &self.config.database_path
    }

    async fn ensure_parent_dir(path: &Path) -> Result<(), RuntimeError> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl RuntimeStore for SqliteRuntimeStore {
    async fn initialize(&self) -> Result<(), RuntimeError> {
        Self::ensure_parent_dir(self.database_path()).await?;
        let _file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.database_path())
            .await?;
        Ok(())
    }

    async fn healthcheck(&self) -> Result<(), RuntimeError> {
        let metadata = tokio::fs::metadata(self.database_path()).await?;
        if metadata.is_file() {
            return Ok(());
        }
        Err(RuntimeError::Bootstrap(format!(
            "database path is not a file: {}",
            self.database_path().display()
        )))
    }
}
