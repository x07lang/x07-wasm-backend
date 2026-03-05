# Repository Guide

## Build and test

- `cargo test`
- `bash scripts/ci/check_phase10.sh`

## Release workflow

- The crate version in `crates/x07-wasm/Cargo.toml` must match the release tag `vX.Y.Z`.
- Reuse the shared release helpers from `x07/scripts/release/` via `.release-tools`; do not add parallel checksum or manifest generators here.
- Release outputs are the `x07-wasm` install archive plus checksums, attestations, and `x07.component.release@0.1.0`.
- Keep vendored `x07-web-ui` assets updated with `python3 scripts/vendor_x07_web_ui.py update --src ../x07-web-ui` when host/runtime integration changes.
