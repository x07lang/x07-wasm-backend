// x07-wasm Phase 4 native CLI component glue.
//
// This file is compiled alongside:
// - x07-generated freestanding C output (program.c + x07.h), which exports x07_solve_v2
// - wit-bindgen C bindings for wasi:cli/command (command.c + command.h + command_component_type.o)
//
// The exported function `exports_wasi_cli_run_run` bridges:
//   wasi:cli/run.run() -> result<(), ()>
// into the Phase 0 ABI:
//   bytes_t x07_solve_v2(uint8_t* arena_mem, uint32_t arena_cap, const uint8_t* input_ptr, uint32_t input_len)

#include "command.h"
#include "x07.h"

#include <stddef.h>
#include <stdint.h>

#ifndef X07_SOLVE_ARENA_CAP_BYTES
#define X07_SOLVE_ARENA_CAP_BYTES (64u * 1024u * 1024u)
#endif

// A hard cap for x07_solve_v2 outputs (enforced before attempting stdout writes).
#ifndef X07_SOLVE_MAX_OUTPUT_BYTES
#define X07_SOLVE_MAX_OUTPUT_BYTES (16u * 1024u * 1024u)
#endif

#ifndef X07_NATIVE_CLI_MAX_STDIN_BYTES
#define X07_NATIVE_CLI_MAX_STDIN_BYTES (1024u * 1024u)
#endif

#ifndef X07_NATIVE_CLI_MAX_STDOUT_BYTES
#define X07_NATIVE_CLI_MAX_STDOUT_BYTES (1024u * 1024u)
#endif

#ifndef X07_NATIVE_CLI_MAX_STDERR_BYTES
#define X07_NATIVE_CLI_MAX_STDERR_BYTES (64u * 1024u)
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

// List/string helpers from wit-bindgen call `free` for post-return allocations.
// In this glue, allocations are managed via a per-call bump arena reset.
void free(void *ptr) {
  (void)ptr;
}

typedef struct x07_buf_t {
  uint8_t *ptr;
  size_t len;
  size_t cap;
} x07_buf_t;

static void x07_buf_init(x07_buf_t *b) {
  b->ptr = NULL;
  b->len = 0;
  b->cap = 0;
}

static void x07_buf_reserve(x07_buf_t *b, size_t add) {
  if (add > ((size_t)-1) - b->len) x07_trap();
  size_t need = b->len + add;
  if (need <= b->cap) return;
  size_t new_cap = b->cap ? b->cap : 4096u;
  while (new_cap < need) {
    if (new_cap > ((size_t)-1) / 2u) new_cap = need;
    else new_cap *= 2u;
  }
  uint8_t *p = (uint8_t *)cabi_realloc(b->ptr, b->cap, 1, new_cap);
  if (p == NULL) x07_trap();
  b->ptr = p;
  b->cap = new_cap;
}

static void x07_buf_push_bytes(x07_buf_t *b, const uint8_t *src, size_t n) {
  x07_buf_reserve(b, n);
  for (size_t i = 0; i < n; i++) {
    b->ptr[b->len + i] = src[i];
  }
  b->len += n;
}

static void x07_write_output_stream_limited(
    wasi_io_streams_borrow_output_stream_t out,
    const uint8_t *bytes,
    size_t len,
    uint64_t max_bytes,
    uint64_t *written,
    bool *ok) {
  if (!*ok) return;
  if (*written + (uint64_t)len > max_bytes) {
    *ok = false;
    return;
  }

  command_list_u8_t chunk;
  chunk.ptr = (uint8_t *)bytes;
  chunk.len = len;

  wasi_io_streams_stream_error_t err;
  if (!wasi_io_streams_method_output_stream_blocking_write_and_flush(out, &chunk, &err)) {
    *ok = false;
    return;
  }
  *written += (uint64_t)len;
}

#define X07_STR_LIT(s) (s), (sizeof(s) - 1u)

static void x07_write_stderr_diag_code_lit(
    const char *code, size_t code_len, uint64_t max_stderr_bytes) {
  wasi_cli_stderr_own_output_stream_t stderr_stream = wasi_cli_stderr_get_stderr();
  wasi_io_streams_borrow_output_stream_t stderr_b =
      wasi_io_streams_borrow_output_stream(stderr_stream);

  uint64_t written = 0;
  bool ok = true;

  static const char prefix[] = "x07-diag-code: ";
  x07_write_output_stream_limited(
      stderr_b, (const uint8_t *)prefix, sizeof(prefix) - 1u, max_stderr_bytes, &written, &ok);
  x07_write_output_stream_limited(
      stderr_b, (const uint8_t *)code, code_len, max_stderr_bytes, &written, &ok);
  static const char suffix[] = "\n";
  x07_write_output_stream_limited(
      stderr_b, (const uint8_t *)suffix, sizeof(suffix) - 1u, max_stderr_bytes, &written, &ok);
}

bool exports_wasi_cli_run_run(void) {
  // Allocate the x07 arena outside the per-call bump reset region.
  if (x07_arena == NULL) {
    x07_arena =
        (uint8_t *)cabi_realloc(NULL, 0, 8, (size_t)X07_SOLVE_ARENA_CAP_BYTES);
    if (x07_arena == NULL) x07_trap();
  }

  // wit-bindgen C wrappers may allocate during run; reset the bump pointer after each call
  // so a host can legally reuse the same component instance for multiple run() invocations.
  uintptr_t call_mark = x07_heap_ptr;

  // Read stdin (bounded).
  wasi_cli_stdin_own_input_stream_t stdin_stream = wasi_cli_stdin_get_stdin();
  wasi_io_streams_borrow_input_stream_t stdin_b =
      wasi_io_streams_borrow_input_stream(stdin_stream);

  x07_buf_t stdin_buf;
  x07_buf_init(&stdin_buf);

  uint64_t read_total = 0;
  for (;;) {
    command_list_u8_t chunk;
    wasi_io_streams_stream_error_t err;
    if (!wasi_io_streams_method_input_stream_blocking_read(
            stdin_b, 4096u, &chunk, &err)) {
      if (err.tag == WASI_IO_STREAMS_STREAM_ERROR_CLOSED) break;
      x07_write_stderr_diag_code_lit(
          X07_STR_LIT("X07WASM_COMPONENT_RUN_STDIN_READ_FAILED"), X07_NATIVE_CLI_MAX_STDERR_BYTES);
      x07_heap_ptr = call_mark;
      return false;
    }
    if (chunk.len == 0) {
      command_list_u8_free(&chunk);
      break;
    }
    if (read_total + (uint64_t)chunk.len > (uint64_t)X07_NATIVE_CLI_MAX_STDIN_BYTES) {
      command_list_u8_free(&chunk);
      x07_write_stderr_diag_code_lit(
          X07_STR_LIT("X07WASM_BUDGET_EXCEEDED_CLI_STDIN"), X07_NATIVE_CLI_MAX_STDERR_BYTES);
      x07_heap_ptr = call_mark;
      return false;
    }
    x07_buf_push_bytes(&stdin_buf, chunk.ptr, chunk.len);
    read_total += (uint64_t)chunk.len;
    command_list_u8_free(&chunk);
  }

  if (stdin_buf.len > 0xffffffffu) {
    x07_write_stderr_diag_code_lit(
        X07_STR_LIT("X07WASM_BUDGET_EXCEEDED_CLI_STDIN"), X07_NATIVE_CLI_MAX_STDERR_BYTES);
    x07_heap_ptr = call_mark;
    return false;
  }

  bytes_t out = x07_solve_v2(
      x07_arena,
      (uint32_t)X07_SOLVE_ARENA_CAP_BYTES,
      (const uint8_t *)stdin_buf.ptr,
      (uint32_t)stdin_buf.len);

  if (out.len > (uint32_t)X07_SOLVE_MAX_OUTPUT_BYTES) x07_trap();

  if (out.len > (uint32_t)X07_NATIVE_CLI_MAX_STDOUT_BYTES) {
    x07_write_stderr_diag_code_lit(
        X07_STR_LIT("X07WASM_BUDGET_EXCEEDED_CLI_STDOUT"), X07_NATIVE_CLI_MAX_STDERR_BYTES);
    x07_heap_ptr = call_mark;
    return false;
  }

  uintptr_t out_ptr = (uintptr_t)out.ptr;
  uintptr_t out_end = out_ptr + (uintptr_t)out.len;
  uintptr_t arena_ptr = (uintptr_t)x07_arena;
  uintptr_t arena_end = arena_ptr + (uintptr_t)X07_SOLVE_ARENA_CAP_BYTES;
  if (out_ptr < arena_ptr || out_end > arena_end) x07_trap();

  wasi_cli_stdout_own_output_stream_t stdout_stream = wasi_cli_stdout_get_stdout();
  wasi_io_streams_borrow_output_stream_t stdout_b =
      wasi_io_streams_borrow_output_stream(stdout_stream);

  uint64_t written = 0;
  bool ok = true;
  const uint8_t *p = (const uint8_t *)out.ptr;
  size_t remaining = (size_t)out.len;
  while (remaining > 0) {
    size_t n = remaining < 4096u ? remaining : 4096u;
    x07_write_output_stream_limited(
        stdout_b, p, n, (uint64_t)X07_NATIVE_CLI_MAX_STDOUT_BYTES, &written, &ok);
    if (!ok) break;
    p += n;
    remaining -= n;
  }

  if (!ok) {
    x07_write_stderr_diag_code_lit(
        X07_STR_LIT("X07WASM_BUDGET_EXCEEDED_CLI_STDOUT"), X07_NATIVE_CLI_MAX_STDERR_BYTES);
    x07_heap_ptr = call_mark;
    return false;
  }

  x07_heap_ptr = call_mark;
  return true;
}
