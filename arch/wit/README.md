# WIT Registry

This repo uses a pinned WIT registry to build and validate WebAssembly components offline.

- Registry: `arch/wit/index.x07wit.json` (`x07.arch.wit.index@0.1.0`)
- Packages:
  - `kind=local`: WIT packages authored in this repo (under `wit/x07/...`)
  - `kind=vendored`: WIT packages vendored into this repo (under `wit/deps/...`)

Validation:

```sh
x07-wasm wit validate --json
```
