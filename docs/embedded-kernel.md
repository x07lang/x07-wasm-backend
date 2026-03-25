# Embedded-kernel workload cells (starter)

`x07-wasm workload pack` supports a starter representation for workload cells that declare:

- `runtime_class=embedded-kernel`
- `scale_class=embedded-kernel`

## What gets emitted

When a service manifest contains an `embedded-kernel` cell, the generated `x07.workload.pack@0.1.0` runtime pack:

- sets `execution_kind=embedded` on that cell, and
- writes an `embedded-kernel.<cell_key>.starter.json` document into the pack output directory.

The cell includes:

- `embedded.kind=embedded-kernel-starter_v1`
- `embedded.manifest` as a `file_digest` that points at the starter manifest.

## Purpose and constraints

The starter manifest is intended to be enough for downstream tooling (platforms and hosted control planes) to:

- recognize embedded-kernel intent,
- present the expected operational shape,
- and carry build/runtime constraints alongside the pack.

It is not a promise that a pack is directly runnable on every target without additional build steps.

