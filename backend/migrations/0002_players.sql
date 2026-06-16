-- 0002_players.sql
-- Player identity, ranked-league membership, and the incremental crawl cursor.
--
-- Riot routing has two systems; keep both straight:
--   platform route (summoner / league-v1): na1, br1, la1, la2, oc1,
--                                           kr, jp1, euw1, eun1, tr1, ru, sg2, tw2, vn2
--   regional route (match-v1):              americas, asia, europe, sea
-- `platform` is stored on the summoner; map platform -> regional route at crawl time.

CREATE TABLE summoners (
    puuid        text PRIMARY KEY,                 -- stable cross-region id
    platform     text NOT NULL,                    -- "kr", "euw1", ...
    game_name    text,
    tag_line     text,
    last_seen_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ON summoners (platform);

-- Snapshot of Master+ league membership, re-populated each crawl from
-- tft-league-v1 challenger / grandmaster / master.
-- This table IS the "Master+ set" used at aggregation time (案Z): a participant's
-- placement is only counted if its puuid is in the Master+ set of the relevant snapshot.
CREATE TABLE ranked_entries (
    puuid         text NOT NULL,
    platform      text NOT NULL,
    queue         text NOT NULL DEFAULT 'RANKED_TFT',
    tier          text NOT NULL,                   -- MASTER | GRANDMASTER | CHALLENGER
    league_points integer NOT NULL DEFAULT 0,
    snapshot_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (puuid, snapshot_at)
);
CREATE INDEX ON ranked_entries (snapshot_at);
CREATE INDEX ON ranked_entries (tier);

-- Per-puuid incremental crawl cursor for matches/by-puuid/{puuid}/ids?startTime=...
-- Lets the crawler pull only new matches and never refetch.
CREATE TABLE crawl_state (
    puuid                 text PRIMARY KEY,
    last_match_start_time bigint,                  -- epoch seconds, passed as startTime
    last_crawled_at       timestamptz,
    matches_seen          integer NOT NULL DEFAULT 0
);
