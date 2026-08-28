# TG4040 compatibility recipes

`compatibility-recipes` is a metadata-only, TG4040-only launcher contract. A
recipe target has an opaque target ID, a local ROM content SHA-256, an exact
core ID/version constraint, a bounded allowlisted settings delta, display/input/
power profile IDs, known issues, project provenance, supersession, and
recipe-layer rollback metadata. It never contains a ROM, BIOS, package,
artifact, executable, script, command, URL, path, payload, or remote-fetch
instruction.

## Trust and privacy

Recipes are repository targets under the narrow delegated TUF `recipes` role;
the role scope is only `recipes/*.json`. `package-trust::TrustStore` verifies
the root, timestamp, snapshot, top-level targets, delegation, expiry,
anti-rollback state, exact target path, signed length, and SHA-256 before the
strict recipe parser and producer validation run. Private signing material is
never stored. The current emulator catalog and core-pack catalog remain
authoritative: blocked or absent cores are unavailable and no core package is
installed or fabricated.

Matching uses only the caller-supplied local content SHA-256 and opaque recipe
target ID. No filename, ROM path, system guess, corpus selection, content
bytes, or user hash is printed, logged, uploaded, cached, or serialized into
diagnostics. The recipe parser rejects duplicate keys, unknown fields, oversized
input, unsafe identifiers/versions/paths/values, secrets, and payload-shaped
metadata.

Display and input references are resolved through their existing producers:
TG4040 stays at logical 1024x768, display precedence remains
`system -> profile -> game -> reset`, and input precedence remains
`built-in -> system -> game -> session`. Recipe settings use the launcher
precedence `device -> system -> core -> folder -> game -> session`. Preview
shows trust tier, opaque target, profile/core changes, every effective setting
before and after, known issues, provenance, and every local collision. Folder,
game, and session choices remain visible and win unless the launcher supplies
an explicit replacement set.

## Launcher lifecycle and rollback

The `sim-launcher::CompatibilityRecipeController` typed surface exposes
`Preview`, explicit `Apply`, and `Rollback`; it is not a shell command UI and
makes no physical-device claim.
Preview has no write effect. Apply validates all preconditions first, creates a
named protected pre-change Save Vault generation, and atomically publishes only
the private named recipe layer under `.brickpro/config/compatibility-recipes`.
Prior recipe-layer state is retained in the vault. Injected publication
failures restore the previous layer. No operation touches `/roms`,
`/data/saves`, or `/data/states`; those protected trees remain byte-identical.
Rollback restores or removes only the recipe layer and does not erase later
local folder, game, or session choices.

## Evidence boundary

Run the generated host fixture evidence with:

```sh
scripts/check-compatibility-recipes
cargo run --locked --release -p compatibility-recipes --bin compatibility-recipes-fixtures -- journey
cargo run --locked -p sim-launcher --bin compatibility-recipe-launcher-fixtures
```

The journey covers signed match, preview/apply/rollback, no-match, target
tampering, expiry, metadata rollback, wrong TG4040 identity, wrong ROM,
unavailable blocked core, invalid and unknown settings, local collisions,
partial publication failure, duplicate keys, size bounds, and unsafe paths.
All metadata and recipe targets are generated synthetic project-owned data.
These checks, host builds, and optional static AArch64 builds are host/static
evidence only; they do not prove hardware, launcher runtime, storage mounts,
core availability, or physical TG4040 behavior.
