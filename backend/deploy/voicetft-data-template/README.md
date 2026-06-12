# voicetft-data

Published TFT stats for the voicetft desktop app. **Generated — do not edit by hand.**
The backend pipeline (`voicetft export` + `run-pipeline.sh`) writes these files and
pushes them here; the desktop app reads them over jsDelivr.

## Layout
```
manifest.json                                  # snapshot id + per-file sha256 (poll this)
set{set}/{patch}/{bracket}/comps.json
                                  units.json
                                  unit_items.json
                                  items.json
                                  augments.json
                                  traits.json
```

## CDN URLs (jsDelivr over this repo)
```
https://cdn.jsdelivr.net/gh/hibikijo0312-hub/voicetft-data@main/manifest.json
https://cdn.jsdelivr.net/gh/hibikijo0312-hub/voicetft-data@main/set17/16.12/master_plus/comps.json
```
The app polls `manifest.json`, compares each file's `sha256` to its local cache, and
downloads only changed files. No user action; no load on the backend VPS.

> jsDelivr caches aggressively (up to ~12h, or 7d on `@version`). Always reference
> `@main` (or `@latest`) for the manifest so updates propagate; purge a path via
> `https://purge.jsdelivr.net/gh/hibikijo0312-hub/voicetft-data@main/manifest.json`
> right after publishing if you need it live immediately.
