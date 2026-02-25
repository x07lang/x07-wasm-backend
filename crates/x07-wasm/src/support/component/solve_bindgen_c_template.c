// x07-wasm Phase 1 solve component glue.
//
// This file is compiled alongside:
// - x07-generated freestanding C output (program.c + x07.h), which exports x07_solve_v2
// - wit-bindgen C bindings for x07:solve/handler (solve.c + solve.h + solve_component_type.o)
//
// The exported function `exports_x07_solve_handler_solve` bridges:
//   x07:solve/handler.solve(list<u8>) -> list<u8>
// into the Phase 0 ABI:
//   bytes_t x07_solve_v2(uint8_t* arena_mem, uint32_t arena_cap, const uint8_t* input_ptr, uint32_t input_len)

#include "solve.h"
#include "x07.h"

#include <stddef.h>
#include <stdint.h>

#ifndef X07_SOLVE_ARENA_CAP_BYTES
#define X07_SOLVE_ARENA_CAP_BYTES (64u * 1024u * 1024u)
#endif

#ifndef X07_SOLVE_MAX_OUTPUT_BYTES
#define X07_SOLVE_MAX_OUTPUT_BYTES (16u * 1024u * 1024u)
#endif

extern unsigned char __heap_base;

static uintptr_t x07_heap_ptr = 0;
static uint8_t *x07_arena = NULL;

static uintptr_t x07_align_up(uintptr_t x, size_t align) {
  if (align <= 1) return x;
  uintptr_t a = (uintptr_t)align;
  return (x + (a - 1u)) & ~(a - 1u);
}

static void x07_trap(void) {
  __builtin_trap();
}

__attribute__((__export_name__("cabi_realloc")))
void *cabi_realloc(void *ptr, size_t old_size, size_t align, size_t new_size) {
  if (new_size == 0) return (void *)align;
  if (align == 0) x07_trap();
  if (x07_heap_ptr == 0) x07_heap_ptr = (uintptr_t)&__heap_base;

  uintptr_t aligned = x07_align_up(x07_heap_ptr, align);
  uintptr_t next = aligned + (uintptr_t)new_size;
  x07_heap_ptr = next;

  if (ptr != NULL && old_size > 0) {
    size_t n = old_size < new_size ? old_size : new_size;
    uint8_t *dst = (uint8_t *)aligned;
    uint8_t *src = (uint8_t *)ptr;
    for (size_t i = 0; i < n; i++) {
      dst[i] = src[i];
    }
  }

  return (void *)aligned;
}

// wit-bindgen's generated post-return frees list results with `free(ptr)`. In
// Phase 1, solve results point into the x07 arena and are not individually
// deallocated.
void free(void *ptr) {
  (void)ptr;
}

void exports_x07_solve_handler_solve(solve_list_u8_t *input, solve_list_u8_t *ret) {
  if (input == NULL || ret == NULL) x07_trap();

  size_t in_len = input->len;
  if (in_len > 0xffffffffu) x07_trap();

  if (x07_arena == NULL) {
    x07_arena = (uint8_t *)cabi_realloc(NULL, 0, 8, (size_t)X07_SOLVE_ARENA_CAP_BYTES);
    if (x07_arena == NULL) x07_trap();
  }

  bytes_t out = x07_solve_v2(
      x07_arena,
      (uint32_t)X07_SOLVE_ARENA_CAP_BYTES,
      (const uint8_t *)input->ptr,
      (uint32_t)in_len);

  if (out.len > (uint32_t)X07_SOLVE_MAX_OUTPUT_BYTES) x07_trap();
  uintptr_t out_ptr = (uintptr_t)out.ptr;
  uintptr_t out_end = out_ptr + (uintptr_t)out.len;
  uintptr_t arena_ptr = (uintptr_t)x07_arena;
  uintptr_t arena_end = arena_ptr + (uintptr_t)X07_SOLVE_ARENA_CAP_BYTES;
  if (out_ptr < arena_ptr || out_end > arena_end) x07_trap();

  ret->ptr = out.ptr;
  ret->len = (size_t)out.len;
}

