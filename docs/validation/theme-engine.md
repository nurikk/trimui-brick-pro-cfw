# Theme engine validation

All commands below run from the repository root. Evidence output belongs in a
fresh caller-owned directory under `/tmp`; it is not a repository artifact.

## Static checks

The workspace lockfile must include the new independent package. The
repository's existing provenance inventory pins the pre-change lockfile hash;
updating that inventory is intentionally outside this ticket's ownership
boundary, so the provenance checks must be rerun after that separately approved
inventory update.

```sh
cargo fmt --check
cargo clippy --locked -p launcher-theme -- -D warnings
cargo build --locked -p launcher-theme --release
python3 -m json.tool schemas/theme-v1.schema.json >/dev/null
for f in themes/default/theme.json themes/samples/*/theme.json; do
  python3 -m json.tool "$f" >/dev/null
 done
PYTHONDONTWRITEBYTECODE=1 python3 scripts/provenance.py check
scripts/test-provenance
git diff --check
```

## Deterministic generated journey

```sh
EVIDENCE_DIR="$(mktemp -d /tmp/launcher-theme.XXXXXX)"
cargo run --locked -p launcher-theme --release -- demo --output "$EVIDENCE_DIR"
python3 - "$EVIDENCE_DIR" <<'PY'
import json, pathlib, struct, sys
root = pathlib.Path(sys.argv[1])
summary = json.loads((root / "summary.json").read_text())
assert len(summary) == 3
for item in summary:
    png = root / pathlib.Path(item["png"]).name
    scene = root / pathlib.Path(item["scene"]).name
    assert png.read_bytes()[:8] == b"\x89PNG\r\n\x1a\n"
    assert struct.unpack(">II", png.read_bytes()[16:24]) == (1024, 768)
    value = json.loads(scene.read_text())
    assert {r["kind"] for r in value["regions"]} == {
        "system-art", "game-list", "box-art-placeholder", "screenshot-placeholder",
        "metadata", "menu", "help-strip", "clock", "battery"
    }
    assert value["settings"]["artworkMode"] in {"system-art", "box-art", "screenshot"}
print("three deterministic scenes and PNG dimensions verified")
PY
```

Run the command again into a second temporary directory and compare the scene
JSON and PNG SHA-256 values. The geometry renderer is deterministic and uses
only synthetic metadata, so both runs must match byte-for-byte.

## Fallback and negative cases

`preview` is the safe integration path. It always emits `result.json`,
`scene.json`, and `preview.png`; invalid input has `fallback: true`,
`theme: "Artbook"`, and a stable kebab-case `reason`. `validate` is the strict
path and exits nonzero. For example:

```sh
cargo run --locked -p launcher-theme --release -- preview \
  --theme /tmp/not-a-theme --output "$EVIDENCE_DIR/missing"
```

A generated negative-case harness should copy a valid `theme.json`, then test
malformed JSON, a duplicate key, an unknown `script` field, an absolute or
parent resource value, a symlink, an unsupported non-JSON file, oversized JSON
or resource declarations, and invalid setting/layout enum or value. Invoke
`preview` for each copy and assert the result has the Artbook fallback and the
expected reason; invoke `validate` to assert nonzero rejection. This exercises
both the file boundary and the parser without adding test fixtures or assets
inside the repository.

## Evidence limits

Native host output is a **host-native userspace simulator**-style logical
renderer result only. It demonstrates parser behavior, semantic scene data,
deterministic output, and PNG dimensions. It does not prove display, GPU,
input, battery, audio, performance, board, or device behavior.

If the checked-in target is installed, run:

```sh
cargo build --locked -p launcher-theme --release --target aarch64-unknown-linux-musl
file target/aarch64-unknown-linux-musl/release/launcher-theme
```

That is compiler/ISA-only evidence. A missing target must be reported rather
than installed. Neither host rendering nor static AArch64 output is physical
TG4040 hardware-in-loop proof; no HIL claim is made by this ticket.
