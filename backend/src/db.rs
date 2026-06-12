use crate::config::Config;
use anyhow::Result;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub async fn pool(cfg: &Config) -> Result<PgPool> {
    Ok(PgPoolOptions::new()
        .max_connections(8)
        .connect(&cfg.database_url)
        .await?)
}
