# Package trust validation

All commands are no-device, local fixture checks. The pinned container uses Debian Bookworm's version-pinned `gcc-aarch64-linux-gnu`, `g++-aarch64-linux-gnu`, and `binutils-aarch64-linux-gnu` to compile AWS-LC C objects for AArch64. Rust links the final `aarch64-unknown-linux-musl` artifact with `rust-lld`; no glibc runtime is linked. `AWS_LC_SYS_NO_ASM` is intentionally unset because aws-lc-sys 0.44.0 permits it only for debug builds. The pinned container builds with Rust 1.85.1 for host and `aarch64-unknown-linux-musl`, then the runtime has no network:

```sh
docker build --network=default -f containers/package-trust/Dockerfile -t brickpro-package-trust .
docker run --rm --network=none brickpro-package-trust
```

The fixture harness prints `PASS` for signed delegated progression, target length/hash failure and retry, typed capability/traversal/case-collision/symlink failure, unsigned/expired/rollback/freeze/clock-uncertainty/corrupt trusted-state failure, blocked core-pack rejection before installation, install/uninstall, interrupted install/uninstall/publication, PortMaster private entrypoint projection, and protected ROM/save/state byte preservation. The build also checks `file`, rejects an ELF interpreter, and rejects `DT_NEEDED` entries for the AArch64 artifact.

Repository checks:

```sh
python3 -m json.tool schemas/package-v1.schema.json >/dev/null
python3 -m json.tool schemas/capabilities-v1.schema.json >/dev/null
python3 scripts/provenance.py generate
python3 scripts/provenance.py check
scripts/test-provenance
git diff --check
```

No Rust/unit-test suite is used. The harness is a real binary running generated synthetic metadata and inert text payloads. It does not access device paths, external CFW source/assets, ROM/BIOS/PortMaster data, or private signing material.
