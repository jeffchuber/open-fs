use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool};

use crate::error::BackendError;
use crate::traits::{Backend, Entry};

/// PostgreSQL storage backend configuration.
#[derive(Debug, Clone)]
pub struct PostgresConfig {
    /// Database connection URL (e.g., postgres://user:pass@host/db)
    pub connection_url: String,
    /// Table name for storing files (default: "ax_files")
    pub table_name: String,
    /// Maximum number of connections in the pool
    pub max_connections: u32,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        PostgresConfig {
            connection_url: String::new(),
            table_name: "ax_files".to_string(),
            max_connections: 5,
        }
    }
}

/// PostgreSQL storage backend.
///
/// This backend stores files in a PostgreSQL database table with the schema:
/// ```sql
/// CREATE TABLE ax_files (
///     path TEXT PRIMARY KEY,
///     content BYTEA NOT NULL,
///     size BIGINT NOT NULL,
///     modified TIMESTAMPTZ NOT NULL DEFAULT NOW()
/// );
/// ```
pub struct PostgresBackend {
    pool: PgPool,
    table_name: String,
}

impl PostgresBackend {
    /// Create a new Postgres backend with the given configuration.
    pub async fn new(config: PostgresConfig) -> Result<Self, BackendError> {
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .connect(&config.connection_url)
            .await
            .map_err(|e| BackendError::Other(format!("Postgres connection failed: {}", e)))?;

        let backend = PostgresBackend {
            pool,
            table_name: config.table_name,
        };

        // Ensure table exists
        backend.ensure_table().await?;

        Ok(backend)
    }

    /// Ensure the storage table exists.
    async fn ensure_table(&self) -> Result<(), BackendError> {
        let query = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                path TEXT PRIMARY KEY,
                content BYTEA NOT NULL,
                size BIGINT NOT NULL,
                modified TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )
            "#,
            self.table_name
        );

        sqlx::query(&query)
            .execute(&self.pool)
            .await
            .map_err(|e| BackendError::Other(format!("Failed to create table: {}", e)))?;

        Ok(())
    }

    /// Normalize path to ensure consistent key format.
    fn normalize_path(path: &str) -> String {
        let path = path.trim_start_matches('/');
        format!("/{}", path)
    }

    /// Extract filename from a path.
    fn filename(path: &str) -> String {
        path.rsplit('/').next().unwrap_or(path).to_string()
    }
}

#[async_trait]
impl Backend for PostgresBackend {
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        let path = Self::normalize_path(path);

        let query = format!(
            "SELECT content FROM {} WHERE path = $1",
            self.table_name
        );

        let row: Option<(Vec<u8>,)> = sqlx::query_as(&query)
            .bind(&path)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| BackendError::Other(format!("Postgres read failed: {}", e)))?;

        match row {
            Some((content,)) => Ok(content),
            None => Err(BackendError::NotFound(path)),
        }
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let path = Self::normalize_path(path);
        let size = content.len() as i64;

        let query = format!(
            r#"
            INSERT INTO {} (path, content, size, modified)
            VALUES ($1, $2, $3, NOW())
            ON CONFLICT (path) DO UPDATE SET
                content = EXCLUDED.content,
                size = EXCLUDED.size,
                modified = NOW()
            "#,
            self.table_name
        );

        sqlx::query(&query)
            .bind(&path)
            .bind(content)
            .bind(size)
            .execute(&self.pool)
            .await
            .map_err(|e| BackendError::Other(format!("Postgres write failed: {}", e)))?;

        Ok(())
    }

    async fn append(&self, path: &str, content: &[u8]) -> Result<(), BackendError> {
        let path = Self::normalize_path(path);

        // Read existing content
        let existing = match self.read(&path).await {
            Ok(data) => data,
            Err(BackendError::NotFound(_)) => Vec::new(),
            Err(e) => return Err(e),
        };

        // Append and write back
        let mut new_content = existing;
        new_content.extend_from_slice(content);

        self.write(&path, &new_content).await
    }

    async fn delete(&self, path: &str) -> Result<(), BackendError> {
        let path = Self::normalize_path(path);

        let query = format!(
            "DELETE FROM {} WHERE path = $1",
            self.table_name
        );

        let result = sqlx::query(&query)
            .bind(&path)
            .execute(&self.pool)
            .await
            .map_err(|e| BackendError::Other(format!("Postgres delete failed: {}", e)))?;

        if result.rows_affected() == 0 {
            return Err(BackendError::NotFound(path));
        }

        Ok(())
    }

    async fn list(&self, path: &str) -> Result<Vec<Entry>, BackendError> {
        let path = Self::normalize_path(path);
        let prefix = if path == "/" {
            "/".to_string()
        } else {
            format!("{}/", path.trim_end_matches('/'))
        };

        // Query all files under this prefix
        let query = format!(
            "SELECT path, size, modified FROM {} WHERE path LIKE $1",
            self.table_name
        );

        let pattern = format!("{}%", prefix);

        let rows: Vec<(String, i64, DateTime<Utc>)> = sqlx::query_as(&query)
            .bind(&pattern)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| BackendError::Other(format!("Postgres list failed: {}", e)))?;

        let mut entries = Vec::new();
        let mut seen_dirs = std::collections::HashSet::new();

        for (file_path, size, modified) in rows {
            // Get the relative path after the prefix
            let relative = file_path
                .strip_prefix(&prefix)
                .unwrap_or(&file_path);

            // Check if this is a direct child or nested
            if let Some(slash_pos) = relative.find('/') {
                // This is a nested file, extract directory name
                let dir_name = &relative[..slash_pos];
                if !dir_name.is_empty() && seen_dirs.insert(dir_name.to_string()) {
                    entries.push(Entry::dir(
                        format!("{}{}", prefix, dir_name),
                        dir_name.to_string(),
                        None,
                    ));
                }
            } else if !relative.is_empty() {
                // Direct child file
                entries.push(Entry::file(
                    file_path.clone(),
                    Self::filename(&file_path),
                    size as u64,
                    Some(modified),
                ));
            }
        }

        // Sort: directories first, then alphabetically
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });

        Ok(entries)
    }

    async fn exists(&self, path: &str) -> Result<bool, BackendError> {
        let path = Self::normalize_path(path);

        let query = format!(
            "SELECT 1 FROM {} WHERE path = $1 LIMIT 1",
            self.table_name
        );

        let row: Option<(i32,)> = sqlx::query_as(&query)
            .bind(&path)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| BackendError::Other(format!("Postgres exists failed: {}", e)))?;

        Ok(row.is_some())
    }

    async fn stat(&self, path: &str) -> Result<Entry, BackendError> {
        let path = Self::normalize_path(path);

        let query = format!(
            "SELECT path, size, modified FROM {} WHERE path = $1",
            self.table_name
        );

        let row: Option<(String, i64, DateTime<Utc>)> = sqlx::query_as(&query)
            .bind(&path)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| BackendError::Other(format!("Postgres stat failed: {}", e)))?;

        match row {
            Some((file_path, size, modified)) => Ok(Entry::file(
                file_path.clone(),
                Self::filename(&file_path),
                size as u64,
                Some(modified),
            )),
            None => Err(BackendError::NotFound(path)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        assert_eq!(PostgresBackend::normalize_path("/file.txt"), "/file.txt");
        assert_eq!(PostgresBackend::normalize_path("file.txt"), "/file.txt");
        assert_eq!(PostgresBackend::normalize_path("/dir/file.txt"), "/dir/file.txt");
    }

    #[test]
    fn test_filename() {
        assert_eq!(PostgresBackend::filename("/dir/file.txt"), "file.txt");
        assert_eq!(PostgresBackend::filename("file.txt"), "file.txt");
        assert_eq!(PostgresBackend::filename("/a/b/c.rs"), "c.rs");
    }
}
