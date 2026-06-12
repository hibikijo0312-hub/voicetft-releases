use anyhow::Result;
use std::path::PathBuf;

pub struct Config {
    pub database_url: String,
    pub riot_api_key: Option<String>,
    pub cdragon_base: String,
    pub export_dir: PathBuf,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            database_url: std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                "postgresql://voicetft:voicetft@localhost:5432/voicetft".into()
            }),
            riot_api_key: std::env::var("RIOT_API_KEY").ok().filter(|k| !k.is_empty()),
            cdragon_base: std::env::var("CDRAGON_BASE")
                .unwrap_or_else(|_| "https://raw.communitydragon.org/latest".into()),
            export_dir: std::env::var("EXPORT_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("export")),
        })
    }
}
