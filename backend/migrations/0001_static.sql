-- 0001_static.sql
-- Static master data sourced from Community Dragon / Data Dragon.
-- IMPORTANT: none of this requires a Riot API key, so it can be populated today.
-- Everything is keyed by `set_id` so multiple TFT sets coexist side by side.

CREATE TABLE sets (
    set_id        integer PRIMARY KEY,            -- e.g. 14
    name          text NOT NULL,                  -- "Cyber City"
    mutator       text,                           -- CDragon set mutator / api set name
    release_patch text,                           -- first patch this set shipped on
    is_active     boolean NOT NULL DEFAULT false,
    created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE patches (
    patch           text PRIMARY KEY,             -- game/data version, e.g. "14.23"
    set_id          integer NOT NULL REFERENCES sets(set_id),
    cdragon_version text,                          -- CDragon/DDragon version string ingested
    started_at      timestamptz,                  -- when this patch went live (best-effort)
    created_at      timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ON patches (set_id);

CREATE TABLE units (
    set_id   integer NOT NULL REFERENCES sets(set_id),
    api_name text NOT NULL,                        -- "TFT14_Jinx" (matches participant.units[].character_id)
    name     text NOT NULL,                        -- display name
    cost     smallint NOT NULL,                    -- 1..5
    traits   text[] NOT NULL DEFAULT '{}',         -- trait api_names this unit carries
    role     text,
    data     jsonb,                                -- raw CDragon blob, for re-derivation
    PRIMARY KEY (set_id, api_name)
);

CREATE TABLE traits (
    set_id      integer NOT NULL REFERENCES sets(set_id),
    api_name    text NOT NULL,                     -- "TFT14_Sniper" (matches participant.traits[].name)
    name        text NOT NULL,
    breakpoints integer[] NOT NULL DEFAULT '{}',   -- active breakpoints, e.g. {2,4,6}
    data        jsonb,
    PRIMARY KEY (set_id, api_name)
);

CREATE TABLE items (
    set_id       integer NOT NULL REFERENCES sets(set_id),
    api_name     text NOT NULL,                    -- "TFT_Item_InfinityEdge" (matches units[].itemNames[])
    name         text NOT NULL,
    components   text[] NOT NULL DEFAULT '{}',     -- component api_names (for completed items)
    is_component boolean NOT NULL DEFAULT false,
    is_emblem    boolean NOT NULL DEFAULT false,
    data         jsonb,
    PRIMARY KEY (set_id, api_name)
);

CREATE TABLE augments (
    set_id   integer NOT NULL REFERENCES sets(set_id),
    api_name text NOT NULL,                        -- "TFT14_Augment_..." (matches participant.augments[])
    name     text NOT NULL,
    tier     smallint,                             -- 1=silver, 2=gold, 3=prismatic (best-effort)
    data     jsonb,
    PRIMARY KEY (set_id, api_name)
);
