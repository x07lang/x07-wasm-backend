# Repository Guide

## Build and test

- `cargo test`
- `bash scripts/ci/check_phase10.sh`
- `bash scripts/ci/check_release_ready.sh`

## Release workflow

- The crate version in `crates/x07-wasm/Cargo.toml` must match the release tag `vX.Y.Z`.
- `scripts/ci/check_release_ready.sh` is the canonical release gate entry point. Keep repo-specific smoke/build checks behind that wrapper.
- Reuse the shared release helpers from `x07/scripts/release/` via `.release-tools`; do not add parallel checksum or manifest generators here.
- Keep `releases/compat/X.Y.Z.json` on the supported core minor line with an upper bound. For the current line, use `x07_core: ">=0.1.58,<0.2.0"` so the published WASM component remains installable across patch toolchain releases.
- Release outputs are the `x07-wasm` install archive plus checksums, attestations, and `x07.component.release@0.1.0`.
- GitHub Actions may not have `CARGO_REGISTRY_TOKEN`; in that case, publish `x07-wasm` locally before calling the release fully published.
- Keep vendored `x07-web-ui` assets updated with `python3 scripts/vendor_x07_web_ui.py update --src ../x07-web-ui` when host/runtime integration changes.
