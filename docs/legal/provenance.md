# Provenance and distribution boundary

`provenance/components.json` is the source of truth. It is a versioned,
deterministically parsed inventory; candidate and artifact IDs use normalized
lowercase identifiers so their SPDX projections cannot collapse distinct IDs.
It records candidate component families and the
separate `distributedArtifacts` list. A **candidate** is evidence about a
possible source or component; it does not authorize shipping bytes. A
**distributed artifact** is one exact staged path with its type, mode, size,
hash or symlink target, source relationship, obligations, and approval.

Only an artifact whose record and candidate both have `status: approved` and
whose candidate has `redistributionStatus: permitted` is projected into:

- `policy/distribution-allowlist.json`;
- `provenance/brickpro-cfw.spdx.json` (SPDX 2.3 JSON); and
- `THIRD_PARTY_NOTICES.md`.

`approved` means the recorded evidence is sufficient for this repository's
engineering gate. `excluded` means the item is deliberately not selected for
this baseline. `blocked` means provenance, licensing, hardware-contract, or
content rights are unresolved or prohibited. `redistributionStatus: permitted`
is a separate required affirmative for a candidate; `not-selected` and
`blocked` never authorize shipping. `NOASSERTION` is never an approval and is
accepted only on blocked candidates.

## Reproduce and audit

From the repository root:

```sh
python3 scripts/provenance.py generate
python3 scripts/provenance.py check
python3 -m json.tool provenance/components.json >/dev/null
python3 -m json.tool policy/distribution-allowlist.json >/dev/null
python3 -m json.tool provenance/brickpro-cfw.spdx.json >/dev/null
scripts/audit-dist /path/to/disposable/staged-root \
  policy/distribution-allowlist.json --inventory provenance/components.json
scripts/test-provenance
```

The generator is the sole writer of the allowlist, SPDX document, and notices.
`check` validates the inventory and fails if any generated output has drifted.
`audit-dist` is read-only and binds the supplied allowlist bytes to the
validated inventory projection before scanning. Required artifact obligations
must name staged paths; it fails closed for unlisted files or links,
metadata/hash mismatches, escaping links, missing artifacts, unmet declared
notice/source paths, non-TG4040 identifiers, and recognizable private
ROM/BIOS/PortMaster paths or content signatures. An empty root with the
current empty allowlist is valid.

## Updating the record

1. For approved or selected material, record an immutable source URL pinned
   to the exact version/commit/tag. A blocked reference or discovery candidate
   may retain a public repository pointer only with an explicit unresolved
   version/reason; that pointer cannot authorize redistribution. Record license
   evidence, attribution, intended use, relationship, obligations, status,
   redistribution status, and a concrete rationale for every candidate.
2. For Rust dependencies, update the exact lockfile representation and hash
   together with authoritative crates.io exact-version evidence. Do not infer
   license facts from a package name.
3. Do not add an artifact for an unbuilt, private, vendor, firmware-only,
   ROM/BIOS, PortMaster, font/theme/artwork, or otherwise unresolved item.
4. For a genuinely approved staged file or symlink, record its exact path,
   mode, size, SHA-256 or link target, and obligations (including required
   staged paths), then run the generator
   and the audit/self-test checks.

This engineering record is not legal advice and does not grant permission to
redistribute third-party material. A later packaging card must add exact,
approved artifact records before anything is shipped.
