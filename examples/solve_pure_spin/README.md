# solve_pure_spin

Deterministic "budget exceeded" fixture for `x07-wasm run --max-fuel`.

This solve-pure program:
- reads a u32 from input[0..4] as N (little-endian),
- loops N times (incrementing acc),
- returns `codec.write_u32_le(acc)`.

Fixtures:
- `tests/fixtures/in_small.bin` (N=5)  -> expected output `out_small.bin` (u32=5).
- `tests/fixtures/in.bin` (N=50_000_000) is intended to exceed fuel when run with a small `--max-fuel`.

Suggested commands:

Build:
  x07-wasm build \
    --project examples/solve_pure_spin/x07.json \
    --profile wasm_release \
    --out dist/solve_pure_spin.wasm \
    --json --quiet-json

Run (success / golden IO):
  x07-wasm run \
    --wasm dist/solve_pure_spin.wasm \
    --input examples/solve_pure_spin/tests/fixtures/in_small.bin \
    --max-fuel 1000000 \
    --output-out /tmp/out_small.bin \
    --json --quiet-json

Run (expected budget exceeded):
  x07-wasm run \
    --wasm dist/solve_pure_spin.wasm \
    --input examples/solve_pure_spin/tests/fixtures/in.bin \
    --max-fuel 10000 \
    --json
