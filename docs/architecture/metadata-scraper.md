# Metadata scraper architecture

## Scope

`metadata-scraper` is a clean-room, host-only synthetic contract for TG4040. The
crate contains no live provider transport, credentials, ROM data, provider
assets, device access, or media byte handling. The checked-in fixture provider
creates deterministic records and HTTPS references for tests only.

## Privacy and opaque identifiers

The public request boundary is keyed by opaque `contentId` and `systemId`.
Identifiers are bounded and reject path syntax. Filename, title, optional hash,
and manual query values are provider inputs; they never appear in
`PublicEvent`. Public events contain only the opaque content ID, provider,
queue state, and a bounded safe reason. Results expose only the fixed typed
metadata and media surface; provider-only fields are not represented.

The logical credential reference, if a future adapter needs one, is
`/data/credentials`. This crate does not read, serialize, log, or send it.

## Query semantics

The fixture provider records query order without transport. When configured and
a hash is present, hash lookup is first. A normalized filename/title lookup is
next when available, followed by an explicit manual query when supplied.
Region and language priority lists select the typed result references. The
fixture data is generated and contains no provider response or artwork bytes.

## Queue and policy

`Queue` persists bounded jobs in a caller-supplied fixture root. States are
exactly `pending`, `running`, `retry`, `succeeded`, `not-found`, `ambiguous`,
`failed`, and `cancelled`. Single and bulk enqueue carry independent metadata
and media overwrite toggles. Pause, resume, cancel, progress, and typed
`enqueue_discovered` are public APIs. Discovery supplies only opaque IDs; it is
not a catalog or filesystem integration.

Automatic discovery enqueue requires Wi-Fi, no suspension, no low battery, no
foreground gameplay, available concurrency, and storage usage within quota.
Dispatch is refused while paused or cancelled. Provider retry-after values are
bounded and take precedence over bounded exponential backoff. Fixture journeys
cover success, not-found, ambiguous, retry, rate-limit, manual search, and
failure paths.

## Checkpoint and recovery

The checkpoint is JSON below the supplied fixture root only. The root and its
checkpoint paths must be real non-symlink paths and reserved `/data`, `/roms`,
and home-directory roots are rejected. Input and checkpoint sizes, strings,
arrays, attempts, reasons, and media URLs are bounded. Unknown JSON fields and
malformed persisted data fail closed.

Writes use a temporary file, `sync_all`, rename, and directory sync. On open,
any persisted `running` job is deterministically changed to `retry` with an
immediate `recovered-running` reason, without discarding its request or
attempt count.

## Results and media handoff

The fixed interchange contract is `scrape-result/v1`, described by
`schemas/scrape-result-v1.schema.json`. It contains opaque IDs, fixture
provider, match status and confidence, typed descriptive metadata, and typed
media references. Media kinds are exactly `box-art`, `screenshot`,
`title-screen`, or `logo`. URLs are validated HTTPS references only: no
userinfo, malformed authority, missing host, localhost, private, loopback,
link-local, multicast, or unspecified IP literals. No bytes are downloaded,
stored, decoded, or cached.

## Deferred non-goals

Live provider adapters, network policy/transport, credential loading, catalog
integration, ROM inspection, media caching, artwork downloads, device/ABI/
loader/filesystem compatibility, and installation are deliberately deferred.
The crate makes no physical-device claim.
