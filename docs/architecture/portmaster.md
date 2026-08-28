# PortMaster boundary

The broker accepts only the catalog's fixed `generated-portmaster` runner and a typed package identity/version. `package-manager` validates the package manifest, private paths, file hashes, capability shape, and the single immutable `immutable/port/launch.sh` entrypoint before activation.

Only that entrypoint and its private `runtime`/`runtime/lib` roots reach the child. ROMs, saves, states, settings, and Save Vault data remain outside package ownership. Interrupted install/update or removal touches only package-owned staging and version directories.
