# Vendored WIT deps

This repo vendors WIT packages to keep builds and validation deterministic and offline.

Current pin:

- Source: `WebAssembly/wasi`
- Tag: `v0.2.8`
- Files copied from: `wasip2/<pkg>/*.wit`

These packages are referenced via `arch/wit/index.x07wit.json` which pins `sha256_tree` digests per package.
