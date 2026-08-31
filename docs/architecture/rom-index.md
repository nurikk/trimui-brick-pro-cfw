# ROM index

`launcher-rom-index/v1` is the single library record. A refresh walks the selected ROM root, ignores hidden and service trees, and atomically replaces `rom-index.json` only after a complete scan. It recognizes nested ROM files, ZIP/7z, CHD, and M3U playlists; CUE playlist members are not games. Paths and names are UTF-8, duplicate display names remain separate entries, and the original filename, derived friendly name, and display-name override are stored separately.

The ID is content-addressed and prefers the existing filesystem device/inode identity when available, so rename/move preserves favorites, collections, history, backlog, metadata, and manual display overrides without a second database. Warm refresh reuses an unchanged path's size/mtime fingerprint; new or changed paths are re-hashed. A cancelled refresh keeps the last index byte-for-byte intact. Its report carries `added`, `removed`, `changed`, and `skipped` counts.

The index is capped at 30,000 entries. Budget: warm refresh should finish within 2 seconds and list navigation stays bounded to 12 rendered rows / 64 search results; initial hashing is explicitly background work and has no launch-frame budget. Text-only, list-with-thumbnail, and artwork-focused views use the same IDs; artwork is resolved through `media-cache` target profiles, never a library-specific cache.

Cleanup must be confirmation-gated and may only remove cache records whose opaque IDs are absent from the just-completed index. ROMs and user artwork are never cleanup candidates.
