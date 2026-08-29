# A/B userspace updates

`update-agent` accepts a closed TG4040 manifest containing the release ID, exact device ID, ordinary HTTPS `artifactUrl`, safe plain `artifactName`, stock-firmware window, userspace ABI, data-schema window, SquashFS payload type, byte size, and SHA-256. It reads only regular files, requires the staged payload filename to match `artifactName`, rejects raw or block-image payloads, and validates all compatibility bounds before writing.

The update is copied into `.brickpro/data/update/staging/<release-id>` and then to the inactive A/B slot with synced temporary files and atomic renames. Explicit confirmation and battery/power gates belong to the caller. A wrong device, size/hash mismatch, malformed manifest, failed boot, or interrupted write leaves the current slot selected; three failed boots trigger automatic rollback. The updater never flashes eMMC automatically.

The release tool packages the manifest and payload into an ordinary deterministic tar archive with no extra authorization metadata.
