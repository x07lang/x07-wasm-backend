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

UPSTREAM_ABI_SNAPSHOT_REL = "arch/host_abi/host_abi.snapshot.json"

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

    new_text = pat.sub(rf"\1{host_abi_hash}\3", text, count=1)
    if new_text == text:
        return

    HOST_ABI_RS.write_text(new_text, encoding="utf-8")


def cmd_update(src_repo: pathlib.Path, source: str) -> int:
    if not (src_repo / ".git").exists():
        raise RuntimeError(f"--src must be a git repo: {src_repo}")

    git_sha = run_git(src_repo, "rev-parse", "HEAD")

    upstream_snapshot = src_repo / UPSTREAM_ABI_SNAPSHOT_REL
    if not upstream_snapshot.is_file():
        raise RuntimeError(f"missing upstream ABI snapshot: {upstream_snapshot}")

    copy_file(upstream_snapshot, VENDORED_ABI_SNAPSHOT)

    snap = read_json(VENDORED_ABI_SNAPSHOT)
    host_abi_hash = snap.get("host_abi_hash", "")
    if not isinstance(host_abi_hash, str) or len(host_abi_hash) != 64:
        raise RuntimeError("vendored host_abi.snapshot.json missing/invalid host_abi_hash")

    sha, n = sha256_file(VENDORED_ABI_SNAPSHOT)

    meta = {
        "source": source,
        "git_sha": git_sha,
        "host_abi_hash": host_abi_hash,
        "digests": {
            UPSTREAM_ABI_SNAPSHOT_REL: {
                "sha256": sha,
                "bytes_len": n,
            }
        },
    }
    write_json(VENDORED_META, meta)

    update_rust_constant(host_abi_hash)

    print(f"updated {VENDORED_ABI_SNAPSHOT.relative_to(REPO_ROOT)} (git_sha={git_sha})")
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

    want_digest = digests[UPSTREAM_ABI_SNAPSHOT_REL]
    got_sha, got_n = sha256_file(VENDORED_ABI_SNAPSHOT)
    got_digest = {"sha256": got_sha, "bytes_len": got_n}
    if want_digest != got_digest:
        print(
            f"FAIL: vendored ABI snapshot digest mismatch: want={want_digest} got={got_digest}",
            file=sys.stderr,
        )
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

        upstream_snapshot = src_repo / UPSTREAM_ABI_SNAPSHOT_REL
        if not upstream_snapshot.is_file():
            print(
                f"FAIL: missing upstream ABI snapshot: {upstream_snapshot}",
                file=sys.stderr,
            )
            return 1

        up_sha, up_n = sha256_file(upstream_snapshot)
        if (up_sha, up_n) != (got_sha, got_n):
            print(
                f"FAIL: upstream snapshot bytes mismatch: upstream={up_sha[:12]}.. vendored={got_sha[:12]}..",
                file=sys.stderr,
            )
            return 1

    print("ok: vendored x07-device-host ABI snapshot matches + rust constant in sync")
    return 0


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(description="Sync/check vendored x07-device-host ABI snapshot.")
    sub = p.add_subparsers(dest="cmd", required=True)

    p_update = sub.add_parser(
        "update", help="Update vendored ABI snapshot from a local x07-device-host repo."
    )
    p_update.add_argument("--src", required=True, type=pathlib.Path, help="Path to x07-device-host repo")
    p_update.add_argument(
        "--source",
        default="https://github.com/x07lang/x07-device-host.git",
        help="Snapshot source identifier",
    )

    p_check = sub.add_parser(
        "check", help="Check vendored snapshot integrity (optionally against src)."
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
