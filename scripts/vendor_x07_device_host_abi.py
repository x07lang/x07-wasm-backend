#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import shutil
import subprocess
import sys


REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]

VENDORED_DIR = REPO_ROOT / "vendor" / "x07-device-host"
VENDORED_ABI_SNAPSHOT = VENDORED_DIR / "host_abi.snapshot.json"
VENDORED_META = VENDORED_DIR / "snapshot.json"
VENDORED_MOBILE_ROOT = VENDORED_DIR / "mobile"

UPSTREAM_ABI_SNAPSHOT_REL = "arch/host_abi/host_abi.snapshot.json"
UPSTREAM_MOBILE_ROOTS = [
    pathlib.PurePosixPath("mobile/ios/template"),
    pathlib.PurePosixPath("mobile/android/template"),
]

HOST_ABI_RS = REPO_ROOT / "crates" / "x07-wasm" / "src" / "device" / "host_abi.rs"


def sha256_file(path: pathlib.Path) -> tuple[str, int]:
    data = path.read_bytes()
    return hashlib.sha256(data).hexdigest(), len(data)


def run_git(src: pathlib.Path, *args: str) -> str:
    out = subprocess.check_output(["git", "-C", str(src), *args], text=True)
    return out.strip()


def read_json(path: pathlib.Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def write_json(path: pathlib.Path, doc: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def copy_file(src: pathlib.Path, dst: pathlib.Path) -> None:
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(src, dst)


def iter_files(root: pathlib.Path) -> list[pathlib.Path]:
    out: list[pathlib.Path] = []
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        if path.name == ".DS_Store" or path.name.startswith("._"):
            continue
        out.append(path.relative_to(root))
    out.sort(key=lambda p: p.as_posix())
    return out


def vendor_path_for_rel(rel: str) -> pathlib.Path:
    if rel == UPSTREAM_ABI_SNAPSHOT_REL:
        return VENDORED_ABI_SNAPSHOT
    return VENDORED_DIR / pathlib.Path(rel)


def update_rust_constant(host_abi_hash: str) -> None:
    if not HOST_ABI_RS.is_file():
        raise RuntimeError(f"missing expected rust file for ABI constant: {HOST_ABI_RS}")

    text = HOST_ABI_RS.read_text(encoding="utf-8")
    import re

    pat = re.compile(
        r'(HOST_ABI_HASH_HEX\s*:\s*&str\s*=\s*\n?\s*")([0-9a-f]{64})(")',
        flags=re.MULTILINE,
    )
    m = pat.search(text)
    if not m:
        raise RuntimeError(
            "HOST_ABI_HASH_HEX constant not found or not in expected form in "
            f"{HOST_ABI_RS}"
        )

    new_text = pat.sub(rf"\g<1>{host_abi_hash}\g<3>", text, count=1)
    if new_text == text:
        return

    HOST_ABI_RS.write_text(new_text, encoding="utf-8")


def collect_upstream_digests(src_repo: pathlib.Path) -> dict[str, dict[str, int | str]]:
    digests: dict[str, dict[str, int | str]] = {}

    upstream_snapshot = src_repo / UPSTREAM_ABI_SNAPSHOT_REL
    if not upstream_snapshot.is_file():
        raise RuntimeError(f"missing upstream ABI snapshot: {upstream_snapshot}")
    sha, n = sha256_file(upstream_snapshot)
    digests[UPSTREAM_ABI_SNAPSHOT_REL] = {"sha256": sha, "bytes_len": n}

    for rel_root in UPSTREAM_MOBILE_ROOTS:
        src_root = src_repo / rel_root
        if not src_root.is_dir():
            raise RuntimeError(f"missing upstream mobile template dir: {src_root}")
        for rel_file in iter_files(src_root):
            upstream_rel = f"{rel_root.as_posix()}/{rel_file.as_posix()}"
            sha, n = sha256_file(src_root / rel_file)
            digests[upstream_rel] = {"sha256": sha, "bytes_len": n}

    return dict(sorted(digests.items()))


def sync_mobile_templates(src_repo: pathlib.Path) -> None:
    if VENDORED_MOBILE_ROOT.exists():
        shutil.rmtree(VENDORED_MOBILE_ROOT)

    for rel_root in UPSTREAM_MOBILE_ROOTS:
        src_root = src_repo / rel_root
        dst_root = VENDORED_DIR / rel_root
        dst_root.mkdir(parents=True, exist_ok=True)
        for rel_file in iter_files(src_root):
            copy_file(src_root / rel_file, dst_root / rel_file)


def cmd_update(src_repo: pathlib.Path, source: str) -> int:
    if not (src_repo / ".git").exists():
        raise RuntimeError(f"--src must be a git repo: {src_repo}")

    git_sha = run_git(src_repo, "rev-parse", "HEAD")
    digests = collect_upstream_digests(src_repo)

    upstream_snapshot = src_repo / UPSTREAM_ABI_SNAPSHOT_REL
    copy_file(upstream_snapshot, VENDORED_ABI_SNAPSHOT)
    sync_mobile_templates(src_repo)

    snap = read_json(VENDORED_ABI_SNAPSHOT)
    host_abi_hash = snap.get("host_abi_hash", "")
    if not isinstance(host_abi_hash, str) or len(host_abi_hash) != 64:
        raise RuntimeError("vendored host_abi.snapshot.json missing/invalid host_abi_hash")

    meta = {
        "source": source,
        "git_sha": git_sha,
        "host_abi_hash": host_abi_hash,
        "digests": digests,
    }
    write_json(VENDORED_META, meta)

    update_rust_constant(host_abi_hash)

    print(f"updated {VENDORED_ABI_SNAPSHOT.relative_to(REPO_ROOT)} (git_sha={git_sha})")
    print(f"updated {VENDORED_MOBILE_ROOT.relative_to(REPO_ROOT)} (git_sha={git_sha})")
    print(f"updated {VENDORED_META.relative_to(REPO_ROOT)} (git_sha={git_sha})")
    print(f"updated {HOST_ABI_RS.relative_to(REPO_ROOT)} (HOST_ABI_HASH_HEX={host_abi_hash})")
    return 0


def cmd_check(src_repo: pathlib.Path | None) -> int:
    if not VENDORED_ABI_SNAPSHOT.is_file():
        print(f"missing vendored ABI snapshot: {VENDORED_ABI_SNAPSHOT}", file=sys.stderr)
        return 1
    if not VENDORED_META.is_file():
        print(f"missing vendored meta snapshot: {VENDORED_META}", file=sys.stderr)
        return 1
    if not HOST_ABI_RS.is_file():
        print(f"missing expected rust file for ABI constant: {HOST_ABI_RS}", file=sys.stderr)
        return 1

    meta = read_json(VENDORED_META)
    digests = meta.get("digests", {})
    host_abi_hash = meta.get("host_abi_hash", "")
    git_sha = meta.get("git_sha", "")

    if not isinstance(git_sha, str) or len(git_sha) < 7:
        print("snapshot.git_sha missing/invalid", file=sys.stderr)
        return 1
    if not isinstance(host_abi_hash, str) or len(host_abi_hash) != 64:
        print("snapshot.host_abi_hash missing/invalid", file=sys.stderr)
        return 1
    if not isinstance(digests, dict) or UPSTREAM_ABI_SNAPSHOT_REL not in digests:
        print("snapshot.digests missing/invalid", file=sys.stderr)
        return 1

    for rel, want_digest in sorted(digests.items()):
        vendor_path = vendor_path_for_rel(rel)
        if not vendor_path.is_file():
            print(f"FAIL: missing vendored file for {rel}: {vendor_path}", file=sys.stderr)
            return 1
        got_sha, got_n = sha256_file(vendor_path)
        got_digest = {"sha256": got_sha, "bytes_len": got_n}
        if want_digest != got_digest:
            print(
                f"FAIL: vendored file digest mismatch for {rel}: want={want_digest} got={got_digest}",
                file=sys.stderr,
            )
            return 1

    extra_vendor_files = []
    for root in [VENDORED_MOBILE_ROOT]:
        if not root.exists():
            print(f"FAIL: missing vendored mobile root: {root}", file=sys.stderr)
            return 1
        for rel_file in iter_files(root):
            rel = f"mobile/{rel_file.as_posix()}"
            if rel not in digests:
                extra_vendor_files.append(rel)
    if extra_vendor_files:
        print(f"FAIL: unexpected vendored mobile files: {extra_vendor_files}", file=sys.stderr)
        return 1

    snap = read_json(VENDORED_ABI_SNAPSHOT)
    snap_hash = snap.get("host_abi_hash", "")
    if snap_hash != host_abi_hash:
        print(
            "FAIL: vendored host_abi.snapshot.json host_abi_hash does not match snapshot.json",
            file=sys.stderr,
        )
        return 1

    rs_text = HOST_ABI_RS.read_text(encoding="utf-8")
    import re

    pat = re.compile(
        r'HOST_ABI_HASH_HEX\s*:\s*&str\s*=\s*\n?\s*"(?P<hash>[0-9a-f]{64})"',
        flags=re.MULTILINE,
    )
    m = pat.search(rs_text)
    if not m:
        print(f"FAIL: missing HOST_ABI_HASH_HEX constant in {HOST_ABI_RS}", file=sys.stderr)
        return 1
    got_rs = m.group("hash")
    if got_rs != host_abi_hash:
        print(
            f"FAIL: HOST_ABI_HASH_HEX does not match vendored host_abi_hash: want={host_abi_hash} got={got_rs}",
            file=sys.stderr,
        )
        return 1

    if src_repo is not None:
        if not (src_repo / ".git").exists():
            print(f"--src must be a git repo: {src_repo}", file=sys.stderr)
            return 1
        head = run_git(src_repo, "rev-parse", "HEAD")
        if head != git_sha:
            print(
                f"FAIL: upstream HEAD mismatch: snapshot={git_sha} upstream={head}",
                file=sys.stderr,
            )
            return 1

        upstream_digests = collect_upstream_digests(src_repo)
        if upstream_digests != digests:
            print("FAIL: upstream digests do not match vendored snapshot.json", file=sys.stderr)
            return 1

    print("ok: vendored x07-device-host ABI + mobile templates match and rust constant in sync")
    return 0


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(
        description="Sync/check vendored x07-device-host ABI snapshot and mobile templates."
    )
    sub = p.add_subparsers(dest="cmd", required=True)

    p_update = sub.add_parser(
        "update",
        help="Update vendored ABI snapshot and mobile templates from a local x07-device-host repo.",
    )
    p_update.add_argument("--src", required=True, type=pathlib.Path, help="Path to x07-device-host repo")
    p_update.add_argument(
        "--source",
        default="https://github.com/x07lang/x07-device-host.git",
        help="Snapshot source identifier",
    )

    p_check = sub.add_parser(
        "check",
        help="Check vendored snapshot integrity (optionally against src).",
    )
    p_check.add_argument("--src", type=pathlib.Path, default=None, help="Path to x07-device-host repo")

    args = p.parse_args(argv)

    if args.cmd == "update":
        return cmd_update(args.src.resolve(), args.source)
    if args.cmd == "check":
        return cmd_check(args.src.resolve() if args.src else None)
    raise RuntimeError(f"unknown cmd: {args.cmd}")


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
