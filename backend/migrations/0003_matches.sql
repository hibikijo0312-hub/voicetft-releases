-- 0003_matches.sql
-- Raw match storage. Per decision 案B, the raw match-v1 JSON is the SINGLE source
-- of truth: there are NO normalized participant/unit/item tables. Aggregation reads
-- raw matches and writes only the summary tables in 0004.
--
-- Partitioned BY LIST (patch): aggregation and retention both operate per patch, so
-- dropping an old patch is a fast DETACH/DROP of one partition. The ingest layer
-- creates one partition per new patch:
--     CREATE TABLE matches_p14_23 PARTITION OF matches FOR VALUES IN ('14.23');

CREATE TABLE matches (
    match_id      text NOT NULL,                  -- "KR_1234567890"
    patch         text NOT NULL,
    set_id        integer NOT NULL,
    region        text NOT NULL,                  -- regional route: americas | asia | europe | sea
    queue_id      integer,                        -- 1100 = ranked TFT
    game_datetime timestamptz NOT NULL,           -- info.game_datetime (ms epoch -> ts)
    game_length   real,
    game_version  text,
    data          jsonb NOT NULL,                 -- full match-v1 payload (metadata + info)
    ingested_at   timestamptz NOT NULL DEFAULT now(),
    -- partition key must be part of every unique key:
    PRIMARY KEY (match_id, patch)
) PARTITION BY LIST (patch);

-- Safety net so an insert never fails before its patch partition exists.
CREATE TABLE matches_default PARTITION OF matches DEFAULT;

-- These become partitioned indexes (one per partition automatically).
CREATE INDEX ON matches (set_id, patch);
CREATE INDEX ON matches (game_datetime);
