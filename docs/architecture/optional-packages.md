# Optional packages

`catalog/packages/optional-modules.json` is a Brick Pro-first, ABI-pinned catalog of reviewed built-in applications, modules, and utilities. Catalog entries name fixed launcher entrypoints; package manifests contain no installer command, shell field, URL, or arbitrary executable payload.

The package manager preflights SKU, ABI, exact library versions, and declared storage before its Save Vault snapshot or package mutation. An activation record is atomically published only after the complete payload is verified; interrupted install/update leaves the prior activation intact.

Packages own only their private immutable/runtime/cache/staging directories. A manifest may explicitly retain individual `writable/` files; update carries them into the new version and uninstall moves them to `data/packages/<id>/`. Global ROMs, saves, states, settings, resume data, and Save Vault data remain protected.

The UI projects installed, update-available, incompatible, and broken package states and limits event logs to 32 lines of 160 characters. Auto-detected ROM matches may be proposed by the launcher, but a proposal never installs or enables a module. Disabled packages retain their data and are excluded from simple launcher mode.
