-- 0004_stats.sql
-- Aggregated, read-optimized statistics. Master+ / all-region for now, but `bracket`
-- and `region` are kept as columns so finer breakdowns can be added later without a
-- schema change.
--
-- Each aggregation run produces one stat_snapshot row; all stat rows reference it.
-- A recompute is atomic: build a new snapshot + its rows, then flip is_current. The
-- exported JSON manifest pins to a snapshot id, so the desktop app always reads a
-- self-consistent set.

CREATE TABLE stat_snapshots (
    id            bigserial PRIMARY KEY,
    set_id        integer NOT NULL,
    patch         text NOT NULL,
    bracket       text NOT NULL DEFAULT 'master_plus',
    region        text NOT NULL DEFAULT 'global',
    games_total   integer NOT NULL DEFAULT 0,     -- # of counted (filtered) participant placements
    matches_total integer NOT NULL DEFAULT 0,
    computed_at   timestamptz NOT NULL DEFAULT now(),
    published_at  timestamptz,                     -- set when exported to the CDN
    is_current    boolean NOT NULL DEFAULT false
);
CREATE UNIQUE INDEX ON stat_snapshots (set_id, patch, bracket, region, computed_at);
CREATE INDEX ON stat_snapshots (is_current) WHERE is_current;

-- Comps. comp_id + name come from the deterministic carry+trait heuristic (案②).
-- `features` keeps the raw signal (carries, core traits, core units) so a future
-- clustering labeler can replace the heuristic WITHOUT a schema change.
CREATE TABLE comp_stats (
    snapshot_id bigint NOT NULL REFERENCES stat_snapshots(id) ON DELETE CASCADE,
    comp_id     text NOT NULL,
    name        text NOT NULL,                    -- e.g. "Jinx Sniper"
    features    jsonb NOT NULL,
    games       integer NOT NULL,
    avg_place   real NOT NULL,
    top4_rate   real NOT NULL,
    win_rate    real NOT NULL,                    -- 1st-place rate
    pick_rate   real NOT NULL,
    PRIMARY KEY (snapshot_id, comp_id)
);

CREATE TABLE unit_stats (
    snapshot_id bigint NOT NULL REFERENCES stat_snapshots(id) ON DELETE CASCADE,
    unit        text NOT NULL,                    -- api_name
    games       integer NOT NULL,
    avg_place   real NOT NULL,
    top4_rate   real NOT NULL,
    win_rate    real NOT NULL,
    pick_rate   real NOT NULL,
    avg_star    real,
    by_star     jsonb,                            -- {"1":{games,avg_place,...}, "2":{...}, "3":{...}}
    PRIMARY KEY (snapshot_id, unit)
);

-- Best-items-on-unit cross table (the high-value "what to build" breakdown).
CREATE TABLE unit_item_stats (
    snapshot_id bigint NOT NULL REFERENCES stat_snapshots(id) ON DELETE CASCADE,
    unit        text NOT NULL,
    item        text NOT NULL,
    games       integer NOT NULL,
    avg_place   real NOT NULL,
    top4_rate   real NOT NULL,
    PRIMARY KEY (snapshot_id, unit, item)
);

CREATE TABLE item_stats (
    snapshot_id bigint NOT NULL REFERENCES stat_snapshots(id) ON DELETE CASCADE,
    item        text NOT NULL,
    games       integer NOT NULL,
    avg_place   real NOT NULL,
    top4_rate   real NOT NULL,
    win_rate    real NOT NULL,
    pick_rate   real NOT NULL,
    PRIMARY KEY (snapshot_id, item)
);

CREATE TABLE augment_stats (
    snapshot_id bigint NOT NULL REFERENCES stat_snapshots(id) ON DELETE CASCADE,
    augment     text NOT NULL,
    games       integer NOT NULL,
    avg_place   real NOT NULL,
    top4_rate   real NOT NULL,
    win_rate    real NOT NULL,
    pick_rate   real NOT NULL,
    by_stage    jsonb,                            -- placement split by stage the augment was offered/picked
    PRIMARY KEY (snapshot_id, augment)
);

CREATE TABLE trait_stats (
    snapshot_id bigint NOT NULL REFERENCES stat_snapshots(id) ON DELETE CASCADE,
    trait       text NOT NULL,
    num_units   smallint NOT NULL,                -- active breakpoint (count of units giving the trait)
    games       integer NOT NULL,
    avg_place   real NOT NULL,
    top4_rate   real NOT NULL,
    PRIMARY KEY (snapshot_id, trait, num_units)
);
