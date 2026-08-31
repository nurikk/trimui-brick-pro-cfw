# Local file manager

`brickpro-file-manager` is a controller-facing maintenance boundary, not a shell or general filesystem browser. It accepts only the allow-listed logical roots: ROMs, BIOS imports, PortMaster data, screenshots, themes, saves export, update sideload, and the USB/network import handoff directories. The handoffs name existing transport ownership; this component does not start USB or network services.

Every user-supplied logical path uses `storage-layout::validate_logical_path` and `resolve_user_path`: absolute paths, traversal, backslashes, symlink components, special files, and every unlisted root are rejected before mutation. The CFW slot is not an addressable root. Hidden entries and operations are blocked unless the controller explicitly enables expert mode; internal staging remains inaccessible in either mode.

Copy and move preflight free space, copy into a hidden same-root staging directory, fsync the file and directory, then rename into place. Cancellation removes the staged result and leaves the source untouched. Conflict choices are skip, deterministic rename, and replace; replace moves the prior destination to recoverable trash and rolls it back if publishing fails. The controller preview supplies the exact logical path, count, and bytes before delete; delete returns the same values in a trash receipt and restore consumes that receipt.

ZIP extraction accepts only regular, non-encrypted stored/deflated ZIP files. It rejects traversal and symlink entries before staging, and caps compressed input and uncompressed output at 512 MiB, entries at 1,000, and expansion at 100:1. Directory browsing scans at most 10,000 user entries and returns 128 sorted rows per page; larger directories fail explicitly rather than producing an unbounded controller list.

Run the controller journey with:

```sh
cargo run -p file-manager --bin brickpro-file-manager -- journey --root "$(mktemp -d)"
```
