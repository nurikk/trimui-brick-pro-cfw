#!/usr/bin/env python3
"""Validate the clean-room provenance inventory and project its approved records."""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import os
import re
import stat
import sys
import tomllib
from pathlib import Path, PurePosixPath
from typing import NoReturn, cast

SCHEMA = "trimui-brick-provenance"
SCHEMA_VERSION = "1.1.0"
SPDX_VERSION = "SPDX-2.3"
STATUSES = {"approved", "excluded", "blocked"}
SPDX_IDENTIFIERS = {
    "0BSD",
    "Apache-2.0",
    "GPL-3.0-only",
    "MIT",
    "NOASSERTION",
    "Unicode-3.0",
    "Unlicense",
    "Zlib",
}
REDISTRIBUTION = {"permitted", "not-selected", "blocked", "not-applicable"}
ID = re.compile(r"^[a-z0-9]+(?:[.-][a-z0-9]+)*$")
ARTIFACT_TYPES = {"regular", "symlink"}
OBLIGATION_KINDS = {"license-notice", "source-availability", "attribution"}
HEX64 = re.compile(r"^[0-9a-f]{64}$")
MODE = re.compile(r"^0[0-7]{3}$")
URL = re.compile(r"^https?://[^\s]+$")
TG_IDENTIFIER = re.compile(r"(?i)TG[0-9]{4}")
PRIVATE_EXTENSIONS = "rom|bin|iso|cue|chd|gba|gbc|gb|nes|sfc|smc|md|gen|sms|pce|pbp|m3u"
PRIVATE_PATH = re.compile(
    rf"(?i)(^|/)(?:roms?|bios|portmaster|ports)(?:/|$)|\.(?:{PRIVATE_EXTENSIONS})$"
)
PRIVATE_CONTENT = re.compile(
    rb"(?i)\b(?:roms?|bios|portmaster)\b|\.(?:rom|bin|iso|cue|chd|gba|gbc|gb|nes|sfc|smc|md|gen|sms|pce|pbp|m3u)\b"
)
TOKEN = re.compile(r"\s*(AND|OR|\(|\)|[A-Za-z0-9.-]+)")
NEXTUI_COMMIT = "ae652648548edf6ab24cbb816cf4e4194e609fb3"
NEXTUI_REPOSITORY = "https://github.com/LoveRetro/NextUI"
NEXTUI_SOURCE_URL = f"{NEXTUI_REPOSITORY}/commit/{NEXTUI_COMMIT}"
NEXTUI_LICENSE_EVIDENCE = re.compile(
    rf"^{re.escape(NEXTUI_REPOSITORY)}/blob/{NEXTUI_COMMIT}/[^?#\s]+$"
)


def fail(message: str) -> NoReturn:
    raise ValueError(message)


def require_object(value: object, label: str) -> dict:
    if not isinstance(value, dict):
        fail(f"{label} must be an object")
    return cast(dict, value)


def require_keys(obj: dict, required: set[str], allowed: set[str], label: str) -> None:
    missing = required - obj.keys()
    unknown = set(obj) - allowed
    if missing:
        fail(f"{label} missing fields: {', '.join(sorted(missing))}")
    if unknown:
        fail(f"{label} has unknown fields: {', '.join(sorted(unknown))}")


def require_string(value: object, label: str, nonempty: bool = True) -> str:
    if not isinstance(value, str) or (nonempty and not value):
        fail(f"{label} must be a {'non-empty ' if nonempty else ''}string")
    return cast(str, value)


def validate_license_expression(value: object, label: str, status: str) -> None:
    expression = require_string(value, label)
    if len(expression) > 256:
        fail(f"{label} is too long")
    if expression == "NOASSERTION":
        if status != "blocked":
            fail(f"{label}: NOASSERTION is only permitted for blocked candidates")
        return
    tokens = []
    position = 0
    while position < len(expression):
        match = TOKEN.match(expression, position)
        if match is None:
            fail(f"{label} is not a valid SPDX expression")
        tokens.append(match.group(1))
        position = match.end()
    cursor = 0

    def primary() -> None:
        nonlocal cursor
        if cursor >= len(tokens):
            fail(f"{label} is not a valid SPDX expression")
        token = tokens[cursor]
        if token == "(":
            cursor += 1
            disjunction()
            if cursor >= len(tokens) or tokens[cursor] != ")":
                fail(f"{label} is not a valid SPDX expression")
            cursor += 1
        elif token not in {"AND", "OR", ")"}:
            if token not in SPDX_IDENTIFIERS or token == "NOASSERTION":
                fail(f"{label} contains an unsupported SPDX identifier")
            cursor += 1
        else:
            fail(f"{label} is not a valid SPDX expression")

    def conjunction() -> None:
        nonlocal cursor
        primary()
        while cursor < len(tokens) and tokens[cursor] == "AND":
            cursor += 1
            primary()

    def disjunction() -> None:
        nonlocal cursor
        conjunction()
        while cursor < len(tokens) and tokens[cursor] == "OR":
            cursor += 1
            conjunction()

    disjunction()
    if cursor != len(tokens):
        fail(f"{label} is not a valid SPDX expression")


def require_nullable_string(obj: dict, field: str, reason: str, label: str) -> None:
    value = obj[field]
    if value is None:
        require_string(obj.get(reason), f"{label}.{reason}")
    else:
        require_string(value, f"{label}.{field}")


def require_url_or_null(obj: dict, field: str, reason: str, label: str) -> None:
    value = obj[field]
    if value is None:
        require_string(obj.get(reason), f"{label}.{reason}")
    elif not isinstance(value, str) or not URL.fullmatch(value):
        fail(f"{label}.{field} must be an http(s) URL or null")


def require_link_target(value: object, label: str) -> str:
    target = require_string(value, label)
    if (
        "\\" in target
        or target.startswith("/")
        or any(part == "" for part in target.split("/"))
    ):
        fail(f"{label} must be a non-empty relative POSIX link target")
    return target


def validate_obligations(
    value: object, label: str, require_paths_for_required: bool = False
) -> None:
    if not isinstance(value, list):
        fail(f"{label} must be a list")
    for i, item in enumerate(cast(list, value)):
        item = require_object(item, f"{label}[{i}]")
        require_keys(
            item,
            {"kind", "required", "detail"},
            {"kind", "required", "detail", "paths"},
            f"{label}[{i}]",
        )
        if not isinstance(item["kind"], str) or item["kind"] not in OBLIGATION_KINDS:
            fail(f"{label}[{i}].kind is unknown")
        if not isinstance(item["required"], bool):
            fail(f"{label}[{i}].required must be boolean")
        require_string(item["detail"], f"{label}[{i}].detail")
        if require_paths_for_required and item["required"] and "paths" not in item:
            fail(f"{label}[{i}].required obligations must name staged paths")
        if "paths" in item:
            if (
                not isinstance(item["paths"], list)
                or not item["paths"]
                or any(not isinstance(p, str) for p in item["paths"])
            ):
                fail(f"{label}[{i}].paths must be a non-empty string list")
            for path in item["paths"]:
                validate_path(path, f"{label}[{i}].paths")


def validate_path(value: object, label: str) -> str:
    path = require_string(value, label)
    if "\\" in path or path.startswith("/"):
        fail(f"{label} must be a relative POSIX path")
    parts = path.split("/")
    if any(part in {"", ".", ".."} for part in parts):
        fail(f"{label} is not normalized")
    if str(PurePosixPath(path)) != path:
        fail(f"{label} is not normalized")
    if PRIVATE_PATH.search(path) and path != "THIRD_PARTY_NOTICES.md":
        fail(f"{label} is a prohibited private-content path")
    for match in TG_IDENTIFIER.finditer(path):
        if match.group(0).upper() != "TG4040":
            fail(f"{label} contains an unapproved target identifier")
    return path


def load_json(path: Path) -> dict:
    try:
        with path.open(encoding="utf-8") as handle:
            return require_object(json.load(handle), str(path))
    except json.JSONDecodeError as exc:
        fail(f"{path}: invalid JSON: {exc}")


def validate_inventory(inventory: dict, inventory_path: Path) -> None:
    require_keys(
        inventory,
        {
            "schema",
            "schemaVersion",
            "repository",
            "lockfile",
            "candidates",
            "distributedArtifacts",
        },
        {
            "schema",
            "schemaVersion",
            "repository",
            "lockfile",
            "candidates",
            "distributedArtifacts",
        },
        "inventory",
    )
    if inventory["schema"] != SCHEMA or inventory["schemaVersion"] != SCHEMA_VERSION:
        fail("inventory schema or schemaVersion is unsupported")
    repo = require_object(inventory["repository"], "repository")
    require_keys(
        repo,
        {"name", "targetSku", "commit", "generatedAt"},
        {"name", "targetSku", "commit", "generatedAt"},
        "repository",
    )
    for key in ("name", "targetSku", "commit", "generatedAt"):
        require_string(repo[key], f"repository.{key}")
    if repo["targetSku"] != "TG4040":
        fail("repository.targetSku must be TG4040")
    lock = require_object(inventory["lockfile"], "lockfile")
    require_keys(
        lock, {"path", "sha256", "packages"}, {"path", "sha256", "packages"}, "lockfile"
    )
    lock_path = inventory_path.parent / require_string(lock["path"], "lockfile.path")
    if not HEX64.fullmatch(require_string(lock["sha256"], "lockfile.sha256")):
        fail("lockfile.sha256 must be lowercase SHA-256")
    if not lock_path.is_file():
        fail(f"lockfile does not exist beside inventory: {lock_path}")
    if hashlib.sha256(lock_path.read_bytes()).hexdigest() != lock["sha256"]:
        fail("Cargo.lock hash differs from the pinned inventory hash")
    if not isinstance(lock["packages"], list):
        fail("lockfile.packages must be a list")
    expected_lock = []
    parsed_lock = {}
    try:
        parsed_lock = tomllib.loads(lock_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        fail(f"cannot parse lockfile: {exc}")
    for package in parsed_lock.get("package", []):
        expected_lock.append(
            {key: package.get(key) for key in ("name", "version", "source", "checksum")}
        )
    if lock["packages"] != expected_lock:
        fail("inventory lockfile.packages is not an exact representation of Cargo.lock")

    candidates = inventory["candidates"]
    artifacts = inventory["distributedArtifacts"]
    if not isinstance(candidates, list) or not isinstance(artifacts, list):
        fail("candidates and distributedArtifacts must be lists")
    candidate_ids: set[str] = set()
    candidates_by_id: dict[str, dict] = {}
    for i, candidate in enumerate(candidates):
        label = f"candidates[{i}]"
        candidate = require_object(candidate, label)
        required = {
            "id",
            "componentClass",
            "name",
            "sourceUrl",
            "version",
            "licenseExpression",
            "licenseEvidenceUrl",
            "copyrightAttribution",
            "intendedUse",
            "status",
            "redistributionStatus",
            "sourceRelationship",
            "obligations",
            "rationale",
        }
        allowed = required | {
            "sourceUrlReason",
            "versionReason",
            "licenseEvidenceReason",
            "licenseEvidence",
            "lockfilePackage",
        }
        require_keys(candidate, required, allowed, label)
        cid = require_string(candidate["id"], f"{label}.id")
        if not ID.fullmatch(cid):
            fail(f"{label}.id must be a normalized lowercase identifier")
        if cid in candidate_ids:
            fail(f"duplicate candidate id: {cid}")
        candidate_ids.add(cid)
        candidates_by_id[cid] = candidate
        for field in (
            "componentClass",
            "name",
            "copyrightAttribution",
            "intendedUse",
            "sourceRelationship",
            "rationale",
        ):
            require_string(candidate[field], f"{label}.{field}")
        require_url_or_null(candidate, "sourceUrl", "sourceUrlReason", label)
        require_nullable_string(candidate, "version", "versionReason", label)
        require_url_or_null(
            candidate, "licenseEvidenceUrl", "licenseEvidenceReason", label
        )
        if cid == "nextui":
            if (
                candidate["version"] != NEXTUI_COMMIT
                or candidate["sourceUrl"] != NEXTUI_SOURCE_URL
            ):
                fail("nextui requires the pinned commit source evidence")
            if (
                not isinstance(candidate["licenseEvidenceUrl"], str)
                or NEXTUI_LICENSE_EVIDENCE.search(candidate["licenseEvidenceUrl"])
                is None
            ):
                fail("nextui requires pinned license evidence at blob/<commit>/...")
        require_string(candidate["licenseExpression"], f"{label}.licenseExpression")
        if (
            not isinstance(candidate["status"], str)
            or candidate["status"] not in STATUSES
        ):
            fail(f"{label}.status is unknown")
        if (
            not isinstance(candidate["redistributionStatus"], str)
            or candidate["redistributionStatus"] not in REDISTRIBUTION
        ):
            fail(f"{label}.redistributionStatus is unknown")
        validate_license_expression(
            candidate["licenseExpression"],
            f"{label}.licenseExpression",
            candidate["status"],
        )
        validate_obligations(candidate["obligations"], f"{label}.obligations")
        if "licenseEvidence" in candidate:
            require_object(candidate["licenseEvidence"], f"{label}.licenseEvidence")
            require_keys(
                candidate["licenseEvidence"],
                {"observedExpression", "basis"},
                {"observedExpression", "basis"},
                f"{label}.licenseEvidence",
            )
            require_string(
                candidate["licenseEvidence"]["observedExpression"],
                f"{label}.licenseEvidence.observedExpression",
            )
            require_string(
                candidate["licenseEvidence"]["basis"], f"{label}.licenseEvidence.basis"
            )
        if "lockfilePackage" in candidate:
            package = require_object(
                candidate["lockfilePackage"], f"{label}.lockfilePackage"
            )
            require_keys(
                package,
                {"name", "version", "source", "checksum"},
                {"name", "version", "source", "checksum"},
                f"{label}.lockfilePackage",
            )
            require_string(package["name"], f"{label}.lockfilePackage.name")
            require_string(package["version"], f"{label}.lockfilePackage.version")
            if package["source"] is not None:
                require_string(package["source"], f"{label}.lockfilePackage.source")
            if package["checksum"] is not None and not HEX64.fullmatch(
                require_string(package["checksum"], f"{label}.lockfilePackage.checksum")
            ):
                fail(f"{label}.lockfilePackage.checksum must be lowercase SHA-256")

    expected_identities = []
    for package in lock["packages"]:
        if not isinstance(package, dict) or set(package) != {
            "name",
            "version",
            "source",
            "checksum",
        }:
            fail("each lockfile package must have exactly name/version/source/checksum")
        expected_identities.append(
            (
                require_string(package["name"], "lockfile package name"),
                require_string(package["version"], "lockfile package version"),
                package["source"],
                package["checksum"],
            )
        )
    represented = []
    for candidate in candidates:
        if "lockfilePackage" in candidate:
            package = candidate["lockfilePackage"]
            represented.append(
                (
                    package["name"],
                    package["version"],
                    package["source"],
                    package["checksum"],
                )
            )
    if len(set(expected_identities)) != len(expected_identities):
        fail("Cargo.lock contains duplicate package identities")
    if len(represented) != len(expected_identities) or Counter(represented) != Counter(
        expected_identities
    ):
        fail("candidate lockfilePackage identities do not exactly cover Cargo.lock")

    artifact_ids: set[str] = set()
    artifact_paths: set[str] = set()
    for i, artifact in enumerate(artifacts):
        label = f"distributedArtifacts[{i}]"
        artifact = require_object(artifact, label)
        required = {
            "id",
            "candidateId",
            "destinationPath",
            "type",
            "mode",
            "linkTarget",
            "buildSourceRelationship",
            "obligations",
            "status",
        }
        require_keys(artifact, required, required, label)
        aid = require_string(artifact["id"], f"{label}.id")
        if not ID.fullmatch(aid):
            fail(f"{label}.id must be a normalized lowercase identifier")
        if aid in artifact_ids:
            fail(f"duplicate artifact id: {aid}")
        artifact_ids.add(aid)
        path = validate_path(artifact["destinationPath"], f"{label}.destinationPath")
        if path in artifact_paths:
            fail(f"duplicate artifact path: {path}")
        artifact_paths.add(path)
        candidate_id = require_string(artifact["candidateId"], f"{label}.candidateId")
        candidate = candidates_by_id.get(candidate_id)
        if candidate is None:
            fail(f"{label}.candidateId does not name a candidate")
        if (
            candidate["status"] != "approved"
            or candidate["redistributionStatus"] != "permitted"
        ):
            fail(
                f"{label} references a candidate that is not approved for redistribution"
            )
        if artifact["status"] != "approved":
            fail(f"{label}.status must be approved for a distributed artifact")
        if (
            not isinstance(artifact["type"], str)
            or artifact["type"] not in ARTIFACT_TYPES
        ):
            fail(f"{label}.type is unknown")
        mode = require_string(artifact["mode"], f"{label}.mode")
        if not MODE.fullmatch(mode):
            fail(f"{label}.mode must be a four-digit octal mode")
        if artifact["type"] == "regular":
            if artifact["linkTarget"] is not None:
                fail(f"{label}.linkTarget must be null for regular files")
        else:
            if artifact["linkTarget"] is None:
                fail(f"{label} symlink requires a linkTarget")
            require_link_target(artifact["linkTarget"], f"{label}.linkTarget")
        require_string(
            artifact["buildSourceRelationship"], f"{label}.buildSourceRelationship"
        )
        validate_obligations(artifact["obligations"], f"{label}.obligations", True)
        candidate_obligations = {
            (item["kind"], item["required"], item["detail"])
            for item in candidate["obligations"]
        }
        artifact_obligations = {
            (item["kind"], item["required"], item["detail"])
            for item in artifact["obligations"]
        }
        if not candidate_obligations <= artifact_obligations:
            fail(f"{label}.obligations omit candidate obligations")


def approved_records(inventory: dict) -> list[tuple[dict, dict]]:
    candidates = {candidate["id"]: candidate for candidate in inventory["candidates"]}
    records = []
    for artifact in sorted(
        inventory["distributedArtifacts"], key=lambda item: item["destinationPath"]
    ):
        candidate = candidates[artifact["candidateId"]]
        records.append((candidate, artifact))
    return records


def json_bytes(value: object) -> bytes:
    return (
        json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"
    ).encode("utf-8")


def render_allowlist(inventory: dict) -> dict:
    artifacts = []
    for candidate, artifact in approved_records(inventory):
        artifacts.append(
            {
                "id": artifact["id"],
                "candidateId": candidate["id"],
                "candidateStatus": candidate["status"],
                "candidateRedistributionStatus": candidate["redistributionStatus"],
                "destinationPath": artifact["destinationPath"],
                "type": artifact["type"],
                "mode": artifact["mode"],
                "linkTarget": artifact["linkTarget"],
                "buildSourceRelationship": artifact["buildSourceRelationship"],
                "obligations": artifact["obligations"],
                "status": artifact["status"],
            }
        )
    return {
        "schema": "trimui-brick-distribution-allowlist",
        "schemaVersion": SCHEMA_VERSION,
        "targetSku": "TG4040",
        "artifacts": artifacts,
    }


def spdx_id(prefix: str, value: str) -> str:
    safe = re.sub(r"[^A-Za-z0-9.-]+", "-", value).strip("-")
    return f"SPDXRef-{prefix}-{safe}"


def render_spdx(inventory: dict, payload: list[dict] | None = None) -> dict:
    records = approved_records(inventory)
    identities = {item["path"]: item for item in payload or []}
    if payload is not None and set(identities) != {
        artifact["destinationPath"] for _, artifact in records
    }:
        fail("candidate payload does not exactly cover approved distributed artifacts")
    packages = []
    files = []
    relationships = []
    seen_candidates = set()
    for candidate, artifact in records:
        package_id = spdx_id("Candidate", candidate["id"])
        file_id = spdx_id("Artifact", artifact["id"])
        if candidate["id"] not in seen_candidates:
            packages.append(
                {
                    "SPDXID": package_id,
                    "name": candidate["name"],
                    "versionInfo": candidate["version"] or "NOASSERTION",
                    "downloadLocation": candidate["sourceUrl"] or "NOASSERTION",
                    "filesAnalyzed": False,
                    "licenseConcluded": candidate["licenseExpression"],
                    "licenseDeclared": candidate["licenseExpression"],
                    "copyrightText": candidate["copyrightAttribution"],
                }
            )
            seen_candidates.add(candidate["id"])
        identity = identities.get(artifact["destinationPath"])
        identity_comment = {
            "artifactId": artifact["id"],
            "type": artifact["type"],
            "mode": artifact["mode"],
            "linkTarget": artifact["linkTarget"],
        }
        if identity is not None:
            identity_comment["byteSize"] = identity["size"]
        file_record = {
            "SPDXID": file_id,
            "fileName": artifact["destinationPath"],
            "licenseConcluded": candidate["licenseExpression"],
            "licenseInfoInFiles": [candidate["licenseExpression"]],
            "copyrightText": candidate["copyrightAttribution"],
            "comment": json.dumps(
                identity_comment, sort_keys=True, separators=(",", ":")
            ),
        }
        if artifact["type"] == "regular":
            if identity is not None:
                file_record["checksums"] = [
                    {"algorithm": "SHA256", "checksumValue": identity["sha256"]}
                ]
        else:
            file_record["fileTypes"] = ["SYMLINK"]
        files.append(file_record)
        relationships.append(
            {
                "spdxElementId": package_id,
                "relationshipType": "CONTAINS",
                "relatedSpdxElement": file_id,
            }
        )
    return {
        "spdxVersion": SPDX_VERSION,
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": "trimui-brick-pro-cfw-distribution",
        "documentNamespace": f"https://github.com/nurikk/trimui-brick-pro-cfw/spdx/{inventory['repository']['commit']}",
        "creationInfo": {
            "created": inventory["repository"]["generatedAt"],
            "creators": ["Tool: scripts/provenance.py"],
        },
        "documentDescribes": list(
            dict.fromkeys(
                spdx_id("Candidate", candidate["id"]) for candidate, _ in records
            )
        ),
        "packages": packages,
        "files": files,
        "relationships": relationships,
    }


def render_notices(inventory: dict) -> str:
    records = approved_records(inventory)
    lines = [
        "# Third-party notices",
        "",
        "Generated from approved distributed artifacts by `scripts/provenance.py`.",
        "",
    ]
    if not records:
        lines.append("No approved distributed artifacts are recorded.")
        return "\n".join(lines) + "\n"
    grouped: dict[str, tuple[dict, list[dict]]] = {}
    for candidate, artifact in records:
        if candidate["id"] not in grouped:
            grouped[candidate["id"]] = (candidate, [])
        grouped[candidate["id"]][1].append(artifact)
    for candidate, artifacts in sorted(
        grouped.values(), key=lambda item: item[0]["id"]
    ):
        lines.extend(
            [
                f"## {candidate['name']}",
                "",
                f"- License: `{candidate['licenseExpression']}`",
                f"- Source: {candidate['sourceUrl'] or 'NOASSERTION'}",
                f"- Attribution: {candidate['copyrightAttribution']}",
                "- Distributed paths:",
            ]
        )
        lines.extend(f"  - `{artifact['destinationPath']}`" for artifact in artifacts)
        lines.append("")
    return "\n".join(lines)


def resolve_outputs(args: argparse.Namespace) -> tuple[Path, Path, Path]:
    root = Path(__file__).resolve().parents[1]
    return (
        Path(args.allowlist or root / "policy/distribution-allowlist.json"),
        Path(args.spdx or root / "provenance/brickpro-cfw.spdx.json"),
        Path(args.notices or root / "THIRD_PARTY_NOTICES.md"),
    )


def generate_or_check(args: argparse.Namespace, check: bool) -> None:
    root = Path(__file__).resolve().parents[1]
    inventory_path = Path(args.inventory or root / "provenance/components.json")
    inventory = load_json(inventory_path)
    validate_inventory(inventory, inventory_path)
    allowlist, spdx, notices = resolve_outputs(args)
    expected = dict(
        zip(
            (allowlist, spdx, notices),
            (
                json_bytes(render_allowlist(inventory)),
                json_bytes(render_spdx(inventory)),
                render_notices(inventory).encode("utf-8"),
            ),
        )
    )
    drift = []
    for path, content in expected.items():
        if check:
            if not path.is_file() or path.read_bytes() != content:
                drift.append(str(path))
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
    if drift:
        fail("generated output drift: " + ", ".join(drift))
    print("checked" if check else "generated", *(str(path) for path in expected))


def validate_allowlist(data: dict) -> None:
    require_keys(
        data,
        {"schema", "schemaVersion", "targetSku", "artifacts"},
        {"schema", "schemaVersion", "targetSku", "artifacts"},
        "allowlist",
    )
    if (
        data["schema"] != "trimui-brick-distribution-allowlist"
        or data["schemaVersion"] != SCHEMA_VERSION
        or data["targetSku"] != "TG4040"
    ):
        fail("allowlist schema or targetSku is unsupported")
    if not isinstance(data["artifacts"], list):
        fail("allowlist.artifacts must be a list")
    ids = set()
    paths = set()
    for i, raw_artifact in enumerate(data["artifacts"]):
        label = f"allowlist.artifacts[{i}]"
        artifact = require_object(raw_artifact, label)
        require_keys(
            artifact,
            {
                "id",
                "candidateId",
                "candidateStatus",
                "candidateRedistributionStatus",
                "destinationPath",
                "type",
                "mode",
                "linkTarget",
                "buildSourceRelationship",
                "obligations",
                "status",
            },
            {
                "id",
                "candidateId",
                "candidateStatus",
                "candidateRedistributionStatus",
                "destinationPath",
                "type",
                "mode",
                "linkTarget",
                "buildSourceRelationship",
                "obligations",
                "status",
            },
            label,
        )
        artifact_id = require_string(artifact["id"], f"{label}.id")
        if not ID.fullmatch(artifact_id):
            fail(f"{label}.id must be a normalized lowercase identifier")
        if artifact_id in ids:
            fail(f"duplicate allowlist id: {artifact['id']}")
        ids.add(artifact_id)
        path = validate_path(artifact["destinationPath"], f"{label}.destinationPath")
        if path in paths:
            fail(f"duplicate allowlist path: {path}")
        paths.add(path)
        if (
            not isinstance(artifact["candidateStatus"], str)
            or not isinstance(artifact["candidateRedistributionStatus"], str)
            or not isinstance(artifact["status"], str)
        ):
            fail(f"{label} has malformed approval status")
        if (
            artifact["candidateStatus"] != "approved"
            or artifact["candidateRedistributionStatus"] != "permitted"
            or artifact["status"] != "approved"
        ):
            fail(f"{label} is not approved for redistribution")
        if (
            not isinstance(artifact["type"], str)
            or artifact["type"] not in ARTIFACT_TYPES
            or not MODE.fullmatch(require_string(artifact["mode"], f"{label}.mode"))
        ):
            fail(f"{label} has invalid type or mode")
        if artifact["type"] == "regular":
            if artifact["linkTarget"] is not None:
                fail(f"{label} has invalid regular-file link target")
        elif artifact["linkTarget"] is None:
            fail(f"{label} has invalid symlink link target")
        if artifact["linkTarget"] is not None:
            require_link_target(artifact["linkTarget"], f"{label}.linkTarget")
        validate_obligations(artifact["obligations"], f"{label}.obligations", True)


def scan_regular_file(path: Path) -> tuple[int, str]:
    digest = hashlib.sha256()
    tail = b""
    size = 0
    elf = False
    with path.open("rb") as handle:
        while chunk := handle.read(64 * 1024):
            if size == 0:
                elf = chunk.startswith(b"\x7fELF")
            size += len(chunk)
            digest.update(chunk)
            window = tail + chunk
            if not elf:
                match = PRIVATE_CONTENT.search(window)
                if match and not (
                    (
                        path.name == "THIRD_PARTY_NOTICES.md"
                        and match.group(0).lower() == b".md"
                    )
                    or (
                        path.name == "compatibility.json"
                        and path.parent.name == "tg4040"
                        and path.parent.parent.name == "platform"
                        and match.group(0).lower() == b"roms"
                    )
                ):
                    fail(f"{path}: private ROM/BIOS/PortMaster content signature")
            tail = window[-64:]
    return size, digest.hexdigest()


def private_or_target_problem(path: str) -> str | None:
    if PRIVATE_PATH.search(path) and path != "THIRD_PARTY_NOTICES.md":
        return "private ROM/BIOS/PortMaster signature"
    for match in TG_IDENTIFIER.finditer(path):
        if match.group(0).upper() != "TG4040":
            return "unapproved target identifier"
    return None


def validate_candidate_manifest(manifest: dict) -> list[dict]:
    require_keys(
        manifest,
        {"schema", "targetSku", "payload", "provenance"},
        set(manifest),
        "candidate manifest",
    )
    require_string(manifest["schema"], "candidate manifest.schema")
    if manifest["targetSku"] != "TG4040":
        fail("candidate manifest.targetSku must be TG4040")
    provenance = require_object(manifest["provenance"], "candidate manifest.provenance")
    require_keys(
        provenance,
        {"inventory", "inventorySha256", "allowlist", "allowlistSha256"},
        {"inventory", "inventorySha256", "allowlist", "allowlistSha256"},
        "candidate manifest.provenance",
    )
    if provenance["inventory"] != "provenance/components.json":
        fail("candidate manifest provenance names an unexpected inventory")
    if provenance["allowlist"] != "policy/distribution-allowlist.json":
        fail("candidate manifest provenance names an unexpected allowlist")
    for field in ("inventorySha256", "allowlistSha256"):
        if not HEX64.fullmatch(
            require_string(provenance[field], f"candidate manifest.provenance.{field}")
        ):
            fail(f"candidate manifest.provenance.{field} must be lowercase SHA-256")
    payload = manifest["payload"]
    if not isinstance(payload, list):
        fail("candidate manifest.payload must be a list")
    paths: set[str] = set()
    for i, raw_item in enumerate(payload):
        label = f"candidate manifest.payload[{i}]"
        item = require_object(raw_item, label)
        require_keys(
            item,
            {"path", "type", "mode", "size", "sha256", "linkTarget"},
            {"path", "type", "mode", "size", "sha256", "linkTarget"},
            label,
        )
        path = validate_path(item["path"], f"{label}.path")
        if path in paths:
            fail(f"duplicate candidate payload path: {path}")
        paths.add(path)
        if item["type"] not in ARTIFACT_TYPES:
            fail(f"{label}.type is unknown")
        if not MODE.fullmatch(require_string(item["mode"], f"{label}.mode")):
            fail(f"{label}.mode must be a four-digit octal mode")
        if not isinstance(item["size"], int) or item["size"] < 0:
            fail(f"{label}.size must be a non-negative integer")
        if item["type"] == "regular":
            if not HEX64.fullmatch(require_string(item["sha256"], f"{label}.sha256")):
                fail(f"{label}.sha256 must be lowercase SHA-256 for regular files")
            if item["linkTarget"] is not None:
                fail(f"{label}.linkTarget must be null for regular files")
        else:
            if item["sha256"] is not None or item["linkTarget"] is None:
                fail(f"{label} symlink requires null sha256 and a linkTarget")
            require_link_target(item["linkTarget"], f"{label}.linkTarget")
    return cast(list[dict], payload)


def verify_checksum_package(
    manifest_path: Path,
    spdx_path: Path,
    inventory: dict,
    checksums_path: Path,
    allowlist_path: Path,
) -> None:
    package_dir = checksums_path.parent
    if checksums_path.is_symlink() or not checksums_path.is_file():
        fail("candidate checksum package must be a regular file")
    if manifest_path.parent != package_dir or spdx_path.parent != package_dir:
        fail("candidate identity artifacts must share the checksum package directory")
    entries: dict[str, str] = {}
    try:
        lines = checksums_path.read_text(encoding="utf-8").splitlines(keepends=True)
    except OSError as exc:
        fail(f"cannot read candidate checksum package: {exc}")
    for line in lines:
        match = re.fullmatch(r"([0-9a-f]{64})  ([^/\\\n]+)\n", line)
        if match is None:
            fail("candidate checksum package has a malformed line")
        digest, name = match.groups()
        if name == checksums_path.name or name in entries:
            fail("candidate checksum package has a duplicate or self entry")
        entries[name] = digest
    required = {manifest_path.name, spdx_path.name, "THIRD_PARTY_NOTICES.md"}
    if not required <= entries.keys():
        fail("candidate checksum package omits a required identity artifact")
    allowlist_location = allowlist_path.absolute()
    actual_files = {
        path.name
        for path in package_dir.iterdir()
        if (path.is_file() or path.is_symlink())
        and path.absolute() != allowlist_location
    }
    if actual_files != set(entries) | {checksums_path.name}:
        fail("candidate checksum package has missing or extra regular files")
    for name, expected_digest in entries.items():
        path = package_dir / name
        if path.is_symlink() or not path.is_file():
            fail(
                f"candidate checksum package names a missing or non-regular file: {name}"
            )
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected_digest:
            fail(f"candidate checksum mismatch: {name}")
    notices = package_dir / "THIRD_PARTY_NOTICES.md"
    if notices.read_bytes() != render_notices(inventory).encode("utf-8"):
        fail("candidate notices drift from the authoritative provenance projection")


def audit(
    root: Path,
    allowlist_path: Path,
    inventory_path: Path,
    manifest_path: Path,
    spdx_path: Path,
    checksums_path: Path,
) -> None:
    if not root.is_dir() or root.is_symlink():
        fail("staged root must be a real directory")
    inventory = load_json(inventory_path)
    validate_inventory(inventory, inventory_path)
    expected_allowlist = json_bytes(render_allowlist(inventory))
    if (
        not allowlist_path.is_file()
        or allowlist_path.read_bytes() != expected_allowlist
    ):
        fail("allowlist does not equal the authoritative inventory projection")
    allowlist = load_json(allowlist_path)
    validate_allowlist(allowlist)
    manifest = load_json(manifest_path)
    payload = validate_candidate_manifest(manifest)
    provenance = manifest["provenance"]
    if (
        hashlib.sha256(inventory_path.read_bytes()).hexdigest()
        != provenance["inventorySha256"]
    ):
        fail("candidate manifest inventory provenance mismatch")
    if (
        hashlib.sha256(allowlist_path.read_bytes()).hexdigest()
        != provenance["allowlistSha256"]
    ):
        fail("candidate manifest allowlist provenance mismatch")
    verify_checksum_package(
        manifest_path, spdx_path, inventory, checksums_path, allowlist_path
    )
    if spdx_path.read_bytes() != json_bytes(render_spdx(inventory, payload)):
        fail("candidate SBOM does not equal the candidate manifest identity projection")
    expected = {item["destinationPath"]: item for item in allowlist["artifacts"]}
    candidate_expected = {item["path"]: item for item in payload}
    if set(candidate_expected) != set(expected):
        fail("candidate manifest does not exactly cover the allowlisted artifacts")
    found: set[str] = set()
    root_real = root.resolve()

    def visit(directory: Path, relative: str) -> None:
        entries = []
        try:
            entries = list(os.scandir(directory))
        except OSError as exc:
            fail(f"cannot read staged directory {directory}: {exc}")
        for entry in sorted(entries, key=lambda item: item.name):
            path = f"{relative}/{entry.name}" if relative else entry.name
            validate_path(path, "staged path")
            problem = private_or_target_problem(path)
            if problem:
                fail(f"{path}: {problem}")
            mode = entry.stat(follow_symlinks=False).st_mode
            if stat.S_ISLNK(mode):
                target = os.readlink(entry.path)
                if not target or os.path.isabs(target):
                    fail(f"{path}: absolute or empty symlink target")
                target_problem = private_or_target_problem(target)
                if target_problem:
                    fail(f"{path}: {target_problem}")
                resolved = (Path(entry.path).parent / target).resolve()
                try:
                    resolved.relative_to(root_real)
                except ValueError:
                    fail(f"{path}: symlink escapes staged root")
                if not resolved.exists():
                    fail(f"{path}: symlink target does not exist")
                if path not in expected:
                    fail(f"{path}: unlisted symlink")
                item = expected[path]
                if (
                    item["type"] != "symlink"
                    or item["linkTarget"] != target
                    or item["mode"] != format(stat.S_IMODE(mode), "04o")
                    or candidate_expected[path]["type"] != "symlink"
                    or candidate_expected[path]["linkTarget"] != target
                    or candidate_expected[path]["mode"]
                    != format(stat.S_IMODE(mode), "04o")
                    or candidate_expected[path]["size"]
                    != entry.stat(follow_symlinks=False).st_size
                ):
                    fail(f"{path}: symlink metadata mismatch")
                found.add(path)
            elif stat.S_ISDIR(mode):
                visit(Path(entry.path), path)
            elif stat.S_ISREG(mode):
                if path not in expected:
                    fail(f"{path}: unlisted regular file")
                item = expected[path]
                if (
                    item["type"] != "regular"
                    or candidate_expected[path]["type"] != "regular"
                ):
                    fail(f"{path}: wrong type")
                if item["mode"] != format(
                    stat.S_IMODE(mode), "04o"
                ) or candidate_expected[path]["mode"] != format(
                    stat.S_IMODE(mode), "04o"
                ):
                    fail(f"{path}: mode mismatch")
                size, digest = scan_regular_file(Path(entry.path))
                if (
                    size != candidate_expected[path]["size"]
                    or digest != candidate_expected[path]["sha256"]
                ):
                    fail(f"{path}: size or SHA-256 mismatch")
                found.add(path)
            else:
                fail(f"{path}: unsupported staged entry type")

    visit(root, "")
    missing = sorted(set(expected) - found)
    if missing:
        fail("missing listed artifact(s): " + ", ".join(missing))
    required_paths = set()
    for item in allowlist["artifacts"]:
        for obligation in item["obligations"]:
            if obligation["required"] and "paths" in obligation:
                required_paths.update(obligation["paths"])
    for path in sorted(required_paths):
        if path not in found:
            fail(f"unmet obligation: required staged path {path} is absent")
    print(f"audit passed: {len(found)} listed artifact(s)")


def main() -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("generate", "check"):
        command = sub.add_parser(name)
        command.add_argument("--inventory")
        command.add_argument("--allowlist")
        command.add_argument("--spdx")
        command.add_argument("--notices")
    command = sub.add_parser("audit")
    command.add_argument("root", type=Path)
    command.add_argument("allowlist", nargs="?", type=Path)
    command.add_argument("--inventory", type=Path)
    command.add_argument("--manifest", type=Path, required=True)
    command.add_argument("--spdx", type=Path, required=True)
    command.add_argument("--checksums", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == "generate":
            generate_or_check(args, False)
        elif args.command == "check":
            generate_or_check(args, True)
        else:
            root = Path(__file__).resolve().parents[1]
            allowlist = args.allowlist or root / "policy/distribution-allowlist.json"
            inventory = args.inventory or root / "provenance/components.json"
            audit(
                args.root,
                allowlist,
                inventory,
                args.manifest,
                args.spdx,
                args.checksums,
            )
        return 0
    except (OSError, ValueError, KeyError) as exc:
        print(f"provenance audit failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
