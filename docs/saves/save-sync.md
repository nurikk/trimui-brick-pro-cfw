# Conflict-aware save synchronization

Save synchronization uses a dedicated exchange/staging namespace and the Save Vault as its only durable authority. Syncthing and WebDAV are transport adapters; neither selects a canonical save.

A candidate is safe to advance automatically only when its recorded ancestry contains the current canonical payload hash. Divergent ancestry, equal timestamps, deletion versus modification, missing/weak validators, and WebDAV `412 Precondition Failed` remain visible conflicts. Timestamps are display evidence only.

Remote metadata and payloads are verified and quarantined before reconciliation. The exchange is never the live save directory and must contain only regular, non-symlink data. Syncthing `.sync-conflict-*` files remain recoverable candidates. WebDAV replacement uses a strong `If-Match` ETag; creation uses `If-None-Match: *`. A failed precondition is not retried unconditionally.

The launcher exposes local and remote device, timestamp, byte size, safe hash prefix, save kind, ancestry prefixes, state, transport outcome, and **Keep Local**, **Keep Remote**, and **Keep Both**. Every explicit resolution commits the remote candidate before selection, retaining both original payload hashes in Save Vault generations. Keep Both retains both generations and leaves the local canonical payload selected; it is not an automatic merge.

Transport work pauses while gameplay or save flush is active. Pending candidates are stored below the exchange `pending` namespace and can be resumed after restart or network failure without blocking local play.

## Secrets

WebDAV secret material is referenced only through the typed `SecretRef` API and must live directly under `/data/secrets/save-sync`. Production creates that directory with mode `0700` and secret files with mode `0600`. Secret values are never serialized or placed in diagnostics, support bundles, screenshots, normal backups, sync payloads, or logs. The simulator uses no credential values.

This implementation intentionally does not merge saves automatically, synchronize ROMs or BIOS files, share saves, collect telemetry, retry an overwrite, acquire credentials, or claim physical TG4040 Wi-Fi evidence. The generated host journey is the validation surface.
