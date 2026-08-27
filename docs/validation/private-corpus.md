# Private acceptance corpus

This is a separate, opt-in acceptance lane for ROM, BIOS, and PortMaster
validation. It is not a simulator lane and it does not change the synthetic
catalog or its evidence contract. The corpus is never copied into the
repository, container image, public evidence, logs, or generated-content
fixtures.

## Privacy boundary

`scripts/private-corpus` is standard-library-only Python. It does not inspect
the corpus unless `TRIMUI_PRIVATE_CORPUS` is explicitly set. Private commands
also require the explicit `TRIMUI_PRIVATE_CORPUS_STATE` environment variable.
The state directory is outside the repository, is created/kept at mode `0700`,
and contains only atomically replaced mode-`0600` files. The CLI prints only a
random generation ID and aggregate status/counts. It never prints a path,
filename, title, hash, content ID, BIOS value, or PortMaster payload.

The public inputs are `corpus/layout.json`, its schema
`corpus/layout.schema.json`, and `sim/contracts/private-corpus.schema.json`.
They define category classifications, runner/core identifiers, BIOS logical
roles, and profile strategies only; they contain no corpus instance. The
layout covers GB, GBC, GBA, Game Gear, Mega Drive, Neo Geo, NES, PS1, Master
System, SNES, PC Engine, Arcade, BIOS, and PortMaster categories. Component
matching is case-insensitive and includes the short generic aliases `GG`,
`MD`, `SMS`, `PCE`, `PCENGINE`, and `ports` alongside the long category
aliases. Matching uses parent directory components and/or the file extension,
never the filename/title or a private relative path.

## Setup and manifest rotation

Use a private shell, never a simulator shell. The normal state path must be
absolute, durable, operator-owned, mode `0700`, and outside the repository.
For example, choose a persistent operator state directory such as
`/var/lib/trimui-private-corpus` and create it with `install -d -m 0700`:

```sh
export TRIMUI_PRIVATE_CORPUS=/private/operator/mounted-corpus
export TRIMUI_PRIVATE_CORPUS_STATE=/var/lib/trimui-private-corpus
scripts/private-corpus --help
scripts/private-corpus rebuild
scripts/private-corpus verify
```

Use an ephemeral directory under `/tmp` only for isolated validation; it is
not the normal persistence or acceptance setup.

`build` and `rebuild` are aliases. A rebuild walks files in stable relative
order, rejects symlinks and malformed/traversal/absolute names, streams a
SHA-256 hash, records size/system/content ID/required runner/required core,
checks ZIP CRCs, and validates every M3U member canonically inside the corpus.
A random generation ID is assigned only after the complete scan succeeds.
The manifest and selections are written as immutable generation records first;
`current.json` is the single mode-`0600` atomic commit pointer to both. Before
that pointer replacement, any scan, hash, ZIP, M3U, or write failure leaves
the prior generation fully resolvable. A changed size or mtime during hashing
also fails the scan. Unreferenced records from an interrupted build are never
loaded.

For an initial aggregate baseline, review the redacted counts returned by
`rebuild` out of band and pass those reviewed values without echoing them:

```sh
scripts/private-corpus baseline accept --reviewed \
  --files "$REVIEWED_FILES" --bytes "$REVIEWED_BYTES" \
  --zip "$REVIEWED_ZIP" --m3u "$REVIEWED_M3U" --chd "$REVIEWED_CHD"
scripts/private-corpus rebuild
```

Once `baseline.json` exists, a count drift is a hard `baseline_drift` failure;
it does not rotate the manifest. Updating the baseline is an explicit,
reviewed operator action, not an automatic acceptance of drift. A missing
corpus, missing state, or missing manifest is reported as structured
`skipped_private_corpus` with exit status zero. Invalid, writable, changed, or
malformed private data is a hard failure.

## Profiles and private curation

The `smoke` profile selects the first sorted entry for each observed system,
which is a small one-per-system set. `compatibility` selects all sorted entries.
Selection is deterministic, uses unique content IDs, and carries each entry's
configured runner/core. PortMaster is selected only from private manifest data;
no title-based public configuration is used.

```sh
scripts/private-corpus profile list
scripts/private-corpus profile inspect --name smoke
scripts/private-corpus profile select --name smoke
scripts/private-corpus profile select --name compatibility
```

For explicit curation, an operator may keep an external, mode-`0600` override
file matching the public `trimui-private-selection-override/v1` shape. Its
`profiles` values are private `contentIds` only; it must not contain paths,
titles, or payloads. Apply it without printing it:

```sh
scripts/private-corpus profile select --name smoke --override /private/operator/reviewed-selection.json
```

Profile curation writes only a new immutable mode-`0600` selections record
with a UUID suffix and atomically replaces `current.json`; it does not rotate
the manifest generation. Reviewed BIOS expectations and the aggregate
baseline therefore remain valid. The override is validated against that
generation's IDs and cannot introduce duplicates or unknown IDs.

## BIOS expectations

BIOS values are intentionally not in Git, the layout, or the manifest
selection criteria. A separately supplied, operator-reviewed expectations
file is kept outside the repository and must be mode `0600` before use. Its
public shape is `trimui-private-bios-expectations/v1`: `entries` contains one
logical `role`, one private `relativePath`, a SHA-256, and an optional size
per BIOS file. The input is copied only as a mode-`0600` atomically replaced
state artifact and is rebound to the current generation:

```sh
scripts/private-corpus expectations init --reviewed --input /private/operator/reviewed-bios.json
scripts/private-corpus expectations verify
scripts/private-corpus verify --require-expectations
```

`update` has the same explicit `--reviewed --input` gate. The tool never
creates or guesses BIOS hashes. Verification requires exact coverage: every
manifest BIOS entry must appear exactly once, every expectation role and
private relative path must be unique and match one BIOS entry, and reviewed
hash/size must match. Duplicate, missing, unmatched, or mismatched entries
fail without printing a role, path, or value.

## Read-only resolver and downstream contract

`resolve` and `probe` are read-only checks for a future simulator or TG4040
HIL caller:

```sh
scripts/private-corpus probe --profile smoke --slot 0
```

They require an existing `current.json` and generation-bound selection, a
canonical resolved regular file inside the corpus root, and matching current
size and streamed SHA-256. On Linux they fail closed unless the longest
covering `/proc/self/mountinfo` mount for both the corpus root and resolved
entry has VFS option `ro` and is the same mount. Inode mode bits may remain
writable on a read-only bind mount. They also recheck ZIP CRC/M3U containment
where applicable. The private
`resolution.json` artifact contains the resolved entry for the downstream
caller; CLI output remains aggregate-only. Missing corpus/state/manifest is
`skipped_private_corpus`; invalid paths, writable roots/files, generation
mismatch, or content drift are hard failures. A caller must treat a skipped
result as “private lane not run”, never as synthetic success.

No host bind-mount helper is shipped: a generic helper cannot safely choose a
host source or destination without widening the trust boundary. An approved
future HIL runner may mount a pre-existing corpus root read-only into its own
private process namespace, with no network, no repository mount, no broad host
mount, and no copy step. It must set the two environment variables, run
`probe`, consume the private resolution artifact locally, and keep it out of
public evidence. The mount/resolver contract is ready; live simulator and
TG4040 HIL integration is downstream work and is not claimed here.

## Validation

The private lane can be checked without private data using a temporary
synthetic corpus: verify the missing-environment skip, a read-write-mount
probe failure, a real read-only-bind probe success with writable inode modes,
a corrupt ZIP CRC, an M3U traversal rejection, and failed rebuild
rollback/atomicity. Registry interruption probes must cover each immutable
record write and the `current.json` pointer write. These checks must use
temporary state outside the repository. The normal lane remains corpus-free:

```sh
unset TRIMUI_PRIVATE_CORPUS TRIMUI_PRIVATE_CORPUS_STATE
scripts/private-corpus verify
./scripts/sim build
```

The normal simulator continues to mount only repository source read-only and
uses only `sim/fixtures/catalog.json`; no private environment variables or
private state are read. A real HIL result requires observed hardware evidence
and cannot be manufactured by this contract.
