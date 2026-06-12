//! Phase 3: Riot data sources behind a trait. `FixtureSource` (no key) drives
//! development/tests end-to-end; `HttpRiotSource` is swapped in when the key arrives.

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

#[derive(Debug, Clone, Deserialize)]
pub struct RankedEntry {
    pub puuid: String,
    pub platform: String,
    pub tier: String,
    #[serde(default)]
    pub league_points: i32,
    #[serde(default)]
    pub game_name: Option<String>,
    #[serde(default)]
    pub tag_line: Option<String>,
}

#[async_trait]
pub trait RiotSource: Send + Sync {
    /// challenger + grandmaster + master for one platform.
    async fn master_plus(&self, platform: &str) -> Result<Vec<RankedEntry>>;
    /// New ranked-TFT match ids for a puuid since `start_time` (epoch seconds).
    async fn match_ids(
        &self,
        route: &str,
        puuid: &str,
        start_time: Option<i64>,
    ) -> Result<Vec<String>>;
    /// Full match-v1 payload.
    async fn match_detail(&self, route: &str, match_id: &str) -> Result<Value>;
}

/// league-v1 lives on the platform host, match-v1 on the regional route host.
pub fn platform_to_route(platform: &str) -> &'static str {
    match platform {
        "na1" | "br1" | "la1" | "la2" | "oc1" => "americas",
        "kr" | "jp1" => "asia",
        "euw1" | "eun1" | "tr1" | "ru" | "me1" => "europe",
        "sg2" | "tw2" | "vn2" => "sea",
        _ => "americas",
    }
}

// ---------------------------------------------------------------- fixtures

pub struct FixtureSource {
    ranked: Vec<RankedEntry>,
    matches: Vec<Value>,
}

impl FixtureSource {
    pub fn new(dir: PathBuf) -> Result<Self> {
        let ranked: Vec<RankedEntry> = serde_json::from_str(
            &std::fs::read_to_string(dir.join("ranked.json")).context("fixtures/ranked.json")?,
        )?;
        let matches: Vec<Value> = serde_json::from_str(
            &std::fs::read_to_string(dir.join("matches.json")).context("fixtures/matches.json")?,
        )?;
        Ok(Self { ranked, matches })
    }
}

#[async_trait]
impl RiotSource for FixtureSource {
    async fn master_plus(&self, platform: &str) -> Result<Vec<RankedEntry>> {
        Ok(self
            .ranked
            .iter()
            .filter(|e| e.platform == platform)
            .cloned()
            .collect())
    }

    async fn match_ids(
        &self,
        _route: &str,
        puuid: &str,
        start_time: Option<i64>,
    ) -> Result<Vec<String>> {
        Ok(self
            .matches
            .iter()
            .filter(|m| {
                let in_match = m["metadata"]["participants"]
                    .as_array()
                    .map(|ps| ps.iter().any(|p| p.as_str() == Some(puuid)))
                    .unwrap_or(false);
                let ts = m["info"]["game_datetime"].as_i64().unwrap_or(0) / 1000;
                in_match && start_time.map(|s| ts > s).unwrap_or(true)
            })
            .filter_map(|m| m["metadata"]["match_id"].as_str().map(String::from))
            .collect())
    }

    async fn match_detail(&self, _route: &str, match_id: &str) -> Result<Value> {
        self.matches
            .iter()
            .find(|m| m["metadata"]["match_id"].as_str() == Some(match_id))
            .cloned()
            .context("fixture match not found")
    }
}

// ---------------------------------------------------------------- live HTTP

/// Token bucket: short-window spacing + long-window cap, both configurable so dev
/// key limits (20/s, 100/2min) can be swapped for production limits without code change.
pub struct RateLimiter {
    min_interval: Duration,
    window: Duration,
    window_cap: usize,
    state: Mutex<LimiterState>,
}

struct LimiterState {
    last: Option<Instant>,
    recent: std::collections::VecDeque<Instant>,
}

impl RateLimiter {
    pub fn new(per_second: u32, window: Duration, window_cap: usize) -> Self {
        Self {
            min_interval: Duration::from_millis(1000 / per_second.max(1) as u64),
            window,
            window_cap,
            state: Mutex::new(LimiterState {
                last: None,
                recent: Default::default(),
            }),
        }
    }

    pub async fn acquire(&self) {
        loop {
            let wait = {
                let mut s = self.state.lock().await;
                let now = Instant::now();
                while let Some(front) = s.recent.front() {
                    if now.duration_since(*front) > self.window {
                        s.recent.pop_front();
                    } else {
                        break;
                    }
                }
                let spacing = s
                    .last
                    .map(|l| self.min_interval.saturating_sub(now.duration_since(l)))
                    .unwrap_or(Duration::ZERO);
                let window_wait = if s.recent.len() >= self.window_cap {
                    self.window
                        .saturating_sub(now.duration_since(*s.recent.front().unwrap()))
                } else {
                    Duration::ZERO
                };
                let wait = spacing.max(window_wait);
                if wait.is_zero() {
                    s.last = Some(now);
                    s.recent.push_back(now);
                    return;
                }
                wait
            };
            tokio::time::sleep(wait).await;
        }
    }
}

pub struct HttpRiotSource {
    client: reqwest::Client,
    key: String,
    limiters: Mutex<HashMap<String, std::sync::Arc<RateLimiter>>>,
}

impl HttpRiotSource {
    pub fn new(key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            key,
            limiters: Mutex::new(HashMap::new()),
        }
    }

    async fn limiter(&self, host: &str) -> std::sync::Arc<RateLimiter> {
        let mut map = self.limiters.lock().await;
        map.entry(host.to_string())
            // dev key defaults; bump via env when the production key lands
            .or_insert_with(|| {
                std::sync::Arc::new(RateLimiter::new(15, Duration::from_secs(120), 90))
            })
            .clone()
    }

    async fn get(&self, host: &str, path: &str) -> Result<Value> {
        let url = format!("https://{host}.api.riotgames.com{path}");
        loop {
            self.limiter(host).await.acquire().await;
            let resp = self
                .client
                .get(&url)
                .header("X-Riot-Token", &self.key)
                .send()
                .await?;
            if resp.status().as_u16() == 429 {
                let retry = resp
                    .headers()
                    .get("Retry-After")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(10);
                tracing::warn!(url, retry, "429, backing off");
                tokio::time::sleep(Duration::from_secs(retry)).await;
                continue;
            }
            return Ok(resp.error_for_status()?.json().await?);
        }
    }
}

#[async_trait]
impl RiotSource for HttpRiotSource {
    async fn master_plus(&self, platform: &str) -> Result<Vec<RankedEntry>> {
        let mut out = Vec::new();
        for (path, tier) in [
            ("/tft/league/v1/challenger", "CHALLENGER"),
            ("/tft/league/v1/grandmaster", "GRANDMASTER"),
            ("/tft/league/v1/master", "MASTER"),
        ] {
            let v = self.get(platform, path).await?;
            // NOTE: verify entry field names against the live API once the key arrives;
            // older league payloads exposed summonerId only, newer ones include puuid.
            for e in v["entries"].as_array().into_iter().flatten() {
                if let Some(puuid) = e["puuid"].as_str() {
                    out.push(RankedEntry {
                        puuid: puuid.to_string(),
                        platform: platform.to_string(),
                        tier: tier.to_string(),
                        league_points: e["leaguePoints"].as_i64().unwrap_or(0) as i32,
                        game_name: None,
                        tag_line: None,
                    });
                }
            }
        }
        Ok(out)
    }

    async fn match_ids(
        &self,
        route: &str,
        puuid: &str,
        start_time: Option<i64>,
    ) -> Result<Vec<String>> {
        let mut path = format!("/tft/match/v1/matches/by-puuid/{puuid}/ids?count=200");
        if let Some(s) = start_time {
            path.push_str(&format!("&startTime={s}"));
        }
        let v = self.get(route, &path).await?;
        Ok(v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(String::from))
            .collect())
    }

    async fn match_detail(&self, route: &str, match_id: &str) -> Result<Value> {
        self.get(route, &format!("/tft/match/v1/matches/{match_id}"))
            .await
    }
}
