use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod aggregate;
mod cdragon;
mod config;
mod crawl;
mod db;
mod export;
mod riot;

#[derive(Parser)]
#[command(name = "voicetft", about = "voicetft TFT stats backend")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Apply pending SQL migrations.
    Migrate,
    /// Fetch CDragon static data (no Riot key) and upsert reference tables.
    StaticRefresh {
        /// Override the auto-detected active set number.
        #[arg(long)]
        set: Option<i32>,
    },
    /// Crawl Master+ players and ingest raw matches.
    Crawl {
        /// "fixture" (no key) or "riot" (requires RIOT_API_KEY).
        #[arg(long, default_value = "fixture")]
        source: String,
        #[arg(long, value_delimiter = ',', default_value = "kr")]
        platforms: Vec<String>,
        #[arg(long, default_value = "fixtures")]
        fixture_dir: PathBuf,
    },
    /// Recompute stats for one patch into a new snapshot (案B: full per-patch recompute).
    Aggregate {
        #[arg(long)]
        patch: String,
        #[arg(long)]
        set: i32,
        #[arg(long, default_value_t = 200)]
        min_games_comp: i64,
        #[arg(long, default_value_t = 50)]
        min_games_pair: i64,
        #[arg(long, default_value_t = 20)]
        min_games_entity: i64,
    },
    /// Write versioned JSON + manifest for a snapshot (default: current).
    Export {
        #[arg(long)]
        snapshot: Option<i64>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    let cfg = config::Config::from_env()?;
    let pool = db::pool(&cfg).await?;

    match cli.cmd {
        Cmd::Migrate => {
            db::MIGRATOR.run(&pool).await?;
            tracing::info!("migrations applied");
        }
        Cmd::StaticRefresh { set } => cdragon::run(&pool, &cfg, set).await?,
        Cmd::Crawl {
            source,
            platforms,
            fixture_dir,
        } => {
            let src: Box<dyn riot::RiotSource> = match source.as_str() {
                "fixture" => Box::new(riot::FixtureSource::new(fixture_dir)?),
                "riot" => Box::new(riot::HttpRiotSource::new(
                    cfg.riot_api_key
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("RIOT_API_KEY not set"))?,
                )),
                other => anyhow::bail!("unknown source: {other}"),
            };
            crawl::run(&pool, src.as_ref(), &platforms).await?;
        }
        Cmd::Aggregate {
            patch,
            set,
            min_games_comp,
            min_games_pair,
            min_games_entity,
        } => {
            aggregate::run(
                &pool,
                &patch,
                set,
                aggregate::Thresholds {
                    comp: min_games_comp,
                    pair: min_games_pair,
                    entity: min_games_entity,
                },
            )
            .await?
        }
        Cmd::Export { snapshot, out } => {
            let out = out.unwrap_or_else(|| cfg.export_dir.clone());
            export::run(&pool, snapshot, &out).await?
        }
    }
    Ok(())
}
