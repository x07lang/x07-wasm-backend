#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
from dataclasses import dataclass


REPO_ROOT = pathlib.Path(__file__).resolve().parents[1]

SNAPSHOT_PATH = REPO_ROOT / "vendor" / "x07-web-ui" / "snapshot.json"
VENDORED_REPO_DIR = REPO_ROOT / "vendor" / "x07-web-ui"
VENDORED_HOST_DIR = REPO_ROOT / "vendor" / "x07-web-ui" / "host"

HOST_FILES = [
    ("host/index.html", VENDORED_HOST_DIR / "index.html"),
    ("host/app-host.mjs", VENDORED_HOST_DIR / "app-host.mjs"),
    ("host/bootstrap.js", VENDORED_HOST_DIR / "bootstrap.js"),
    ("host/main.mjs", VENDORED_HOST_DIR / "main.mjs"),
    ("host/host.snapshot.json", VENDORED_HOST_DIR / "host.snapshot.json"),
]

WIT_FILES = [
    (
        "wit/x07/web_ui/0.2.0/web-ui-app.wit",
        REPO_ROOT / "wit" / "x07" / "web_ui" / "0.2.0" / "web-ui-app.wit",
    ),
]

TREE_DIRS = [
    (
        "host/tests",
        VENDORED_REPO_DIR / "host" / "tests",
    ),
    (
        "packages/std-web-ui/0.1.2/modules",
        VENDORED_REPO_DIR / "packages" / "std-web-ui" / "0.1.2" / "modules",
    ),
    (
        "packages/std-web-ui/0.1.3/modules",
        VENDORED_REPO_DIR / "packages" / "std-web-ui" / "0.1.3" / "modules",
    ),
    (
        "packages/std-web-ui/0.1.4/modules",
        VENDORED_REPO_DIR / "packages" / "std-web-ui" / "0.1.4" / "modules",
    ),
    (
        "packages/std-web-ui/0.1.5/modules",
        VENDORED_REPO_DIR / "packages" / "std-web-ui" / "0.1.5" / "modules",
    ),
    (
        "packages/std-web-ui/0.1.6/modules",
        VENDORED_REPO_DIR / "packages" / "std-web-ui" / "0.1.6" / "modules",
    ),
    (
        "packages/std-web-ui/0.1.7/modules",
        VENDORED_REPO_DIR / "packages" / "std-web-ui" / "0.1.7" / "modules",
    ),
    (
        "packages/std-web-ui/0.1.8/modules",
        VENDORED_REPO_DIR / "packages" / "std-web-ui" / "0.1.8" / "modules",
    ),
    (
        "packages/std-web-ui/0.1.9/modules",
        VENDORED_REPO_DIR / "packages" / "std-web-ui" / "0.1.9" / "modules",
    ),
    (
        "packages/std-web-ui/0.2.0/modules",
        VENDORED_REPO_DIR / "packages" / "std-web-ui" / "0.2.0" / "modules",
    ),
    (
        "examples/web_ui_counter",
        VENDORED_REPO_DIR / "examples" / "web_ui_counter",
    ),
    (
        "examples/web_ui_form",
        VENDORED_REPO_DIR / "examples" / "web_ui_form",
    ),
]


@dataclass(frozen=True)
class FileDigest:
    relpath: str
    sha256: str
    bytes_len: int

    def as_json(self) -> dict:
        return {"sha256": self.sha256, "bytes_len": self.bytes_len}


def sha256_file(path: pathlib.Path) -> FileDigest:
    data = path.read_bytes()
    return FileDigest(
        relpath=str(path),
        sha256=hashlib.sha256(data).hexdigest(),
        bytes_len=len(data),
    )


def run_git(src: pathlib.Path, *args: str) -> str:
    out = subprocess.check_output(["git", "-C", str(src), *args], text=True)
    return out.strip()


def read_snapshot(path: pathlib.Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def write_snapshot(path: pathlib.Path, doc: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def ensure_parent(path: pathlib.Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)


def copy_file(src: pathlib.Path, dst: pathlib.Path) -> None:
    ensure_parent(dst)
    shutil.copy2(src, dst)


def compute_vendor_files() -> dict[str, pathlib.Path]:
    out: dict[str, pathlib.Path] = {}

    for rel, dst in HOST_FILES:
        if not dst.is_file():
            raise RuntimeError(f"missing vendored file: {dst}")
        out[rel] = dst

    for rel, dst in WIT_FILES:
        if not dst.is_file():
            raise RuntimeError(f"missing synced WIT file: {dst}")
        out[rel] = dst

    for src_rel_dir, dst_dir in TREE_DIRS:
        if not dst_dir.is_dir():
            raise RuntimeError(f"missing vendored dir: {dst_dir}")
        paths = sorted([p for p in dst_dir.rglob("*") if p.is_file()])
        for path in paths:
            rel = path.relative_to(dst_dir).as_posix()
            if rel == "":
                raise RuntimeError(f"unexpected empty relpath for {path}")
            out[f"{src_rel_dir}/{rel}"] = path

    return out


def compute_vendor_digests() -> dict[str, dict]:
    out: dict[str, dict] = {}
    for rel, dst in sorted(compute_vendor_files().items()):
        d = sha256_file(dst)
        out[rel] = {"sha256": d.sha256, "bytes_len": d.bytes_len}
    return out


def copy_tree(src: pathlib.Path, dst: pathlib.Path) -> None:
    if dst.exists():
        shutil.rmtree(dst)
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(src, dst, copy_function=shutil.copy2)


def cmd_update(src_repo: pathlib.Path, source: str) -> int:
    if not (src_repo / ".git").exists():
        raise RuntimeError(f"--src must be a git repo: {src_repo}")

    git_sha = run_git(src_repo, "rev-parse", "HEAD")

    for rel, dst in HOST_FILES:
        src = src_repo / rel
        if not src.is_file():
            raise RuntimeError(f"missing upstream file: {src}")
        copy_file(src, dst)

    for rel, dst in WIT_FILES:
        src = src_repo / rel
        if not src.is_file():
            raise RuntimeError(f"missing upstream file: {src}")
        copy_file(src, dst)

    for rel, dst in TREE_DIRS:
        src = src_repo / rel
        if not src.is_dir():
            raise RuntimeError(f"missing upstream dir: {src}")
        copy_tree(src, dst)

    digests = compute_vendor_digests()

    doc = {
        "source": source,
        "git_sha": git_sha,
        "digests": digests,
    }
    write_snapshot(SNAPSHOT_PATH, doc)
    print(f"updated {SNAPSHOT_PATH.relative_to(REPO_ROOT)} (git_sha={git_sha})")
    return 0


def compare_file(a: pathlib.Path, b: pathlib.Path) -> tuple[bool, str]:
    da = sha256_file(a)
    db = sha256_file(b)
    if da.sha256 != db.sha256 or da.bytes_len != db.bytes_len:
        return (
            False,
            f"{a} != {b} (sha256 {da.sha256[:12]}.. vs {db.sha256[:12]}..)",
        )
    return True, ""


def cmd_check(src_repo: pathlib.Path | None) -> int:
    if not SNAPSHOT_PATH.is_file():
        print(f"missing snapshot: {SNAPSHOT_PATH}", file=sys.stderr)
        return 1

    snap = read_snapshot(SNAPSHOT_PATH)
    git_sha = snap.get("git_sha", "")
    digests = snap.get("digests", {})

    if not isinstance(git_sha, str) or len(git_sha) < 7:
        print("snapshot.git_sha missing/invalid", file=sys.stderr)
        return 1
    if not isinstance(digests, dict) or not digests:
        print("snapshot.digests missing/invalid", file=sys.stderr)
        return 1

    failures: list[str] = []

    vendor_files = compute_vendor_files()
    want_vendor = {rel: sha256_file(dst).as_json() for rel, dst in vendor_files.items()}
    for rel, want in digests.items():
        got = want_vendor.get(rel)
        if got is None:
            failures.append(f"missing vendored digest entry for {rel!r}")
            continue
        if want != got:
            failures.append(f"digest mismatch for {rel!r}: want={want} got={got}")

    for rel in want_vendor.keys():
        if rel not in digests:
            failures.append(f"missing snapshot digest entry for {rel!r}")

    if src_repo is not None:
        if not (src_repo / ".git").exists():
            print(f"--src must be a git repo: {src_repo}", file=sys.stderr)
            return 1
        head = run_git(src_repo, "rev-parse", "HEAD")
        if head != git_sha:
            failures.append(f"upstream HEAD mismatch: snapshot={git_sha} upstream={head}")

        for rel, dst in sorted(vendor_files.items()):
            src = src_repo / rel
            ok, msg = compare_file(src, dst)
            if not ok:
                failures.append(msg)

    if failures:
        for f in failures:
            print(f"FAIL: {f}", file=sys.stderr)
        return 1

    print("ok: vendored x07-web-ui snapshot matches")
    return 0


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(description="Sync/check vendored x07-web-ui snapshots.")
    sub = p.add_subparsers(dest="cmd", required=True)

    p_update = sub.add_parser("update", help="Update vendored files from a local x07-web-ui repo.")
    p_update.add_argument("--src", required=True, type=pathlib.Path, help="Path to x07-web-ui repo")
    p_update.add_argument(
        "--source",
        default="https://github.com/x07lang/x07-web-ui.git",
        help="Snapshot source identifier",
    )

    p_check = sub.add_parser("check", help="Check vendored snapshot integrity (optionally against src).")
    p_check.add_argument("--src", type=pathlib.Path, default=None, help="Path to x07-web-ui repo")

    args = p.parse_args(argv)

    if args.cmd == "update":
        return cmd_update(args.src.resolve(), args.source)
    if args.cmd == "check":
        return cmd_check(args.src.resolve() if args.src else None)
    raise RuntimeError(f"unknown cmd: {args.cmd}")


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
