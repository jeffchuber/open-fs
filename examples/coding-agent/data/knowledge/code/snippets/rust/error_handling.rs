//! Error handling patterns for Rust applications.

use std::fmt;
use thiserror::Error;

/// Application-specific error type using thiserror.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result type alias for convenience.
pub type Result<T> = std::result::Result<T, AppError>;

/// Convert AppError to HTTP status code (for web frameworks).
impl AppError {
    pub fn status_code(&self) -> u16 {
        match self {
            AppError::NotFound(_) => 404,
            AppError::Unauthorized(_) => 401,
            AppError::Validation(_) => 400,
            AppError::Database(_) => 500,
            AppError::Io(_) => 500,
            AppError::Internal(_) => 500,
        }
    }
}

/// Example usage with the ? operator.
pub async fn get_user(id: i64, pool: &sqlx::PgPool) -> Result<User> {
    let user = sqlx::query_as!(User, "SELECT * FROM users WHERE id = $1", id)
        .fetch_optional(pool)
        .await?  // Automatically converts sqlx::Error to AppError
        .ok_or_else(|| AppError::NotFound(format!("User {} not found", id)))?;

    Ok(user)
}

/// Pattern: Early return with context.
pub async fn process_order(order_id: i64, pool: &sqlx::PgPool) -> Result<()> {
    let order = get_order(order_id, pool)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to fetch order: {}", e)))?;

    if order.status != "pending" {
        return Err(AppError::Validation("Order is not pending".into()));
    }

    // Process the order...
    Ok(())
}
