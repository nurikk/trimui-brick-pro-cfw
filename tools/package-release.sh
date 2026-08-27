#!/bin/sh
set -eu

usage() {
    printf '%s\n' 'usage: tools/package-release.sh --manifest FILE --payload FILE --signing-key EXTERNAL_PRIVATE_KEY --out ABSOLUTE_EMPTY_DIR' >&2
    exit 64
}
[ "$#" -eq 8 ] || usage
[ "$1" = --manifest ] || usage
MANIFEST=$2
[ "$3" = --payload ] || usage
PAYLOAD=$4
[ "$5" = --signing-key ] || usage
SIGNING_KEY=$6
[ "$7" = --out ] || usage
OUT=$8
case "$OUT" in /*) ;; *) usage ;; esac
[ -f "$MANIFEST" ] && [ -f "$PAYLOAD" ] && [ -f "$SIGNING_KEY" ] && [ ! -L "$MANIFEST" ] && [ ! -L "$PAYLOAD" ] && [ ! -L "$SIGNING_KEY" ] || {
    echo 'package-release: package inputs must be regular non-symlink files' >&2
    exit 2
}
case "$PAYLOAD" in *.[aA][wW][iI][mM][gG] | *.[rR][aA][wW] | *.[iI][mM][gG])
    echo 'package-release: raw, .awimg, .img, .raw, and block-image payloads are forbidden' >&2
    exit 2
    ;;
esac
[ -d "$OUT" ] || {
    echo 'package-release: output must already exist' >&2
    exit 2
}
[ -z "$(find "$OUT" -mindepth 1 -maxdepth 1 -print -quit)" ] || {
    echo 'package-release: output must be empty' >&2
    exit 2
}
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
PUBLIC_KEY=$ROOT/keys/update.pub
command -v minisign >/dev/null 2>&1 || {
    echo 'package-release: minisign-verifier-unavailable' >&2
    exit 2
}
[ -f "$PUBLIC_KEY" ] && [ ! -L "$PUBLIC_KEY" ] || {
    echo 'package-release: checked-in public key is missing or invalid' >&2
    exit 2
}
TRUSTED=$(
    python3 - "$MANIFEST" "$PAYLOAD" <<'PY'
import hashlib, json, pathlib, re, sys
manifest, payload = map(pathlib.Path, sys.argv[1:])
raw = manifest.read_bytes()
data = json.loads(raw)
expected = {"$schema", "manifestVersion", "deviceId", "releaseId", "releaseSequence", "stockFirmware", "userspaceAbi", "dataSchema", "payloadType", "payloadSize", "payloadSha256", "trustedComment"}
if set(data) != expected or raw != (json.dumps(data, indent=2, sort_keys=True) + "\n").encode():
    raise SystemExit("manifest is not closed canonical v1")
if data["deviceId"] != "TG4040" or data["payloadType"] != "squashfs-userspace":
    raise SystemExit("package is not a TG4040 userspace package")
if not re.fullmatch(r"[a-z0-9](?:[a-z0-9.-]{0,46}[a-z0-9])?", data["releaseId"]):
    raise SystemExit("release ID is outside bounds")
identity = dict(data)
identity.pop("trustedComment")
identity_digest = hashlib.sha256((json.dumps(identity, indent=2, sort_keys=True) + "\n").encode()).hexdigest()
comment = f"project=trimui-brick-pro-cfw; target=tg4040; release={data['releaseId']}; sequence={data['releaseSequence']}; payload-sha256={data['payloadSha256']}; manifest-sha256={identity_digest}"
if data["trustedComment"] != comment:
    raise SystemExit("manifest trusted comment is not bound")
if data["payloadSize"] != payload.stat().st_size or data["payloadSha256"] != hashlib.sha256(payload.read_bytes()).hexdigest() or payload.read_bytes()[:4] != b"hsqs":
    raise SystemExit("payload size, SHA-256, or SquashFS header is invalid")
print(comment)
PY
)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/brickpro-release.XXXXXX")
trap 'rm -rf "$WORK"' EXIT HUP INT TERM
SIGNATURE=$WORK/manifest.minisig
minisign -Sm "$MANIFEST" -s "$SIGNING_KEY" -x "$SIGNATURE" -t "$TRUSTED" >/dev/null
minisign -Vm "$MANIFEST" -x "$SIGNATURE" -p "$PUBLIC_KEY" >/dev/null
python3 - "$MANIFEST" "$PAYLOAD" "$SIGNATURE" "$WORK" <<'PY'
import json, pathlib, shutil, sys
manifest, payload, signature, work = map(pathlib.Path, sys.argv[1:])
data = json.loads(manifest.read_text())
release = data["releaseId"]
out = work / release
out.mkdir()
shutil.copyfile(manifest, out / "manifest.json")
shutil.copyfile(payload, out / "payload.squashfs")
shutil.copyfile(signature, out / "manifest.minisig")
(out / "interface.json").write_text(json.dumps({"schema":"brickpro-update-interface/v1","actions":["boot-current","boot-previous","discard-unactivated-staging"]}, indent=2, sort_keys=True) + "\n")
print(release)
PY
RELEASE=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["releaseId"])' "$MANIFEST")
tar -C "$WORK" --format=ustar --sort=name --mtime='UTC 1970-01-01' -cf "$OUT/update-$RELEASE.tar" "$RELEASE"
sha256sum "$OUT/update-$RELEASE.tar"
