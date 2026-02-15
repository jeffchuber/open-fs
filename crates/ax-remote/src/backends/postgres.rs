use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{postgres::PgPoolOptions, PgPool};

use ax_config::Secret;
use ax_core::{Backend, Entry, BackendError};

/// Validate that a table name is safe for use in SQL identifiers.
fn validate_table_name(name: &str) -> Result<(), BackendError> {
    if name.is_empty() {
        return Err(BackendError::Other("Table name cannot be empty".to_string()));
    }
    if name.len() > 63 {
        return Err(BackendError::Other(format!(
            "Table name '{}' exceeds maximum length of 63 characters",
            name
        )));
    }
    let is_valid = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !is_valid {
        return Err(BackendError::Other(format!(
            "Invalid table name '{}': must match ^[a-zA-Z_][a-zA-Z0-9_]*$",
            name
        )));
    }
    Ok(())
}

/// PostgreSQL storage backend configuration.
#[derive(Debug, Clone)]
pub struct PostgresConfig {
    pub connection_url: Secret,
    pub table_name: String,
    pub max_connections: u32,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        PostgresConfig {
            connection_url: Secret::new(""),
            table_name: "ax_files".to_string(),
            max_connections: 5,
        }
    }
}

/// PostgreSQL storage backend.
pub struct PostgresBackend {
    pool: PgPool,
    table_name: String,
}

impl PostgresBackend {
    pub async fn new(config: PostgresConfig) -> Result<Self, BackendError> {
        validate_table_name(&config.table_name)?;

        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .connect(config.connection_url.expose())
            .await
            .map_err(|e| BackendError::Other(format!("Postgres connection failed: {}", e)))?;

        let backend = PostgresBackend {
            pool,
            table_name: config.table_name,
        };

        backend.ensure_table().await?;
        Ok(backend)
    }

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

    fn normalize_path(path: &str) -> String {
        let path = path.trim_start_matches('/');
        format!("/{}", path)
    }

    fn filename(path: &str) -> String {
        path.rsplit('/').next().unwrap_or(path).to_string()
    }
}

#[async_trait]
impl Backend for PostgresBackend {
    async fn read(&self, path: &str) -> Result<Vec<u8>, BackendError> {
        let path = Self::normalize_path(path);
        let query = format!("SELECT content FROM {} WHERE path = $1", self.table_name);
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
            r#"INSERT INTO {} (path, content, size, modified) VALUES ($1, $2, $3, NOW())
            ON CONFLICT (path) DO UPDATE SET content = EXCLUDED.content, size = EXCLUDED.size, modified = NOW()"#,
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
        let existing = match self.read(&path).await {
            Ok(data) => data,
            Err(BackendError::NotFound(_)) => Vec::new(),
            Err(e) => return Err(e),
        };
        let mut new_content = existing;
        new_content.extend_from_slice(content);
        self.write(&path, &new_content).await
    }

    async fn delete(&self, path: &str) -> Result<(), BackendError> {
        let path = Self::normalize_path(path);
        let query = format!("DELETE FROM {} WHERE path = $1", self.table_name);
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
        let query = format!("SELECT path, size, modified FROM {} WHERE path LIKE $1", self.table_name);
        let pattern = format!("{}%", prefix);
        let rows: Vec<(String, i64, DateTime<Utc>)> = sqlx::query_as(&query)
            .bind(&pattern)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| BackendError::Other(format!("Postgres list failed: {}", e)))?;

        let mut entries = Vec::new();
        let mut seen_dirs = std::collections::HashSet::new();

        for (file_path, size, modified) in rows {
            let relative = file_path.strip_prefix(&prefix).unwrap_or(&file_path);
            if let Some(slash_pos) = relative.find('/') {
                let dir_name = &relative[..slash_pos];
                if !dir_name.is_empty() && seen_dirs.insert(dir_name.to_string()) {
                    entries.push(Entry::dir(
                        format!("{}{}", prefix, dir_name),
                        dir_name.to_string(),
                        None,
                    ));
                }
            } else if !relative.is_empty() {
                entries.push(Entry::file(
                    file_path.clone(),
                    Self::filename(&file_path),
                    size as u64,
                    Some(modified),
                ));
            }
        }

        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        });

        Ok(entries)
    }

    async fn exists(&self, path: &str) -> Result<bool, BackendError> {
        let path = Self::normalize_path(path);
        let query = format!("SELECT 1 FROM {} WHERE path = $1 LIMIT 1", self.table_name);
        let row: Option<(i32,)> = sqlx::query_as(&query)
            .bind(&path)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| BackendError::Other(format!("Postgres exists failed: {}", e)))?;
        Ok(row.is_some())
    }

    async fn stat(&self, path: &str) -> Result<Entry, BackendError> {
        let path = Self::normalize_path(path);
        let query = format!("SELECT path, size, modified FROM {} WHERE path = $1", self.table_name);
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
    }

    #[test]
    fn test_valid_table_names() {
        assert!(validate_table_name("ax_files").is_ok());
        assert!(validate_table_name("_private").is_ok());
    }

    #[test]
    fn test_sql_injection_drop_table() {
        assert!(validate_table_name("\"; DROP TABLE --").is_err());
    }
}
