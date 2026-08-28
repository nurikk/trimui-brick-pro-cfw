# Save Vault

`save-vault` is the typed recovery boundary for SRAM, save files, state files, and explicitly declared state files. Its source-relative allowlist is limited to `saves/`, `states/`, and `declared/`; ROM, BIOS, absolute, traversal, and symlink paths are rejected.

A generation contains a versioned manifest, SHA-256 content IDs, immutable content objects, and a durable commit marker. Objects and manifests are written and synced in staging before generation rename and current-pointer publication. Invalid, incomplete, corrupt, or quarantined generations are never current. Source files are read and checked before and after copying, so a race is a failed snapshot rather than live-data mutation.

Production `SaveVault::new` uses 0700 directories and 0600 files. `for_simulator` is the explicit generated-evidence constructor and uses 0777/0644. These modes are not claims about removable FAT/exFAT storage, which has no POSIX mode contract. Package, update, recipe, and broker boundaries take a typed pre-operation snapshot; a failed prerequisite blocks the operation.

The launcher exposes only aggregate history and a sanitized restore preview through typed session-broker methods. Restore requires a separate confirmation, snapshots current live data first, verifies temporary replacements, and preserves the prior live bytes on cancellation or failure. System rollback does not select or restore a Save Vault generation.
