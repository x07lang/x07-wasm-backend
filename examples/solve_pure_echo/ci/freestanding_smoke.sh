#!/usr/bin/env bash
set -euo pipefail

ROOT="examples/solve_pure_echo"
OUT="target/solve_pure_echo"
mkdir -p "${OUT}"

# 1) Build freestanding C (exports x07_solve_v2)
x07 build \
  --project "${ROOT}/x07.json" \
  --out "${OUT}/program.c" \
  --emit-c-header "${OUT}/x07.h" \
  --freestanding

# 2) Compile harness + generated C
clang -O2 \
  -I"${OUT}" \
  -o "${OUT}/test_embed" \
  "${ROOT}/c/test_embed.c" \
  "${OUT}/program.c"

# 3) Golden checks
"${OUT}/test_embed" "${ROOT}/tests/fixtures/in_empty.bin"  "${ROOT}/tests/fixtures/out_empty.bin"
"${OUT}/test_embed" "${ROOT}/tests/fixtures/in_hello.bin"  "${ROOT}/tests/fixtures/out_hello.bin"
"${OUT}/test_embed" "${ROOT}/tests/fixtures/in_binary.bin" "${ROOT}/tests/fixtures/out_binary.bin"

echo "solve_pure_echo: PASS"

