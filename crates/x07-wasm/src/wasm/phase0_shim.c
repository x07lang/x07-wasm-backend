// Minimal libc shims for Phase 0 freestanding WASM builds.
//
// X07's freestanding C output may reference a small subset of libc APIs in
// trap/debug paths. Phase 0 does not link a full libc, so these definitions
// provide deterministic fallbacks without importing WASI.

#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>

char* getenv(const char* name) {
  (void)name;
  return (char*)0;
}

static void shim_write_char(char* s, size_t n, size_t* pos, char c) {
  if (n == 0) return;
  if (*pos + 1 < n) {
    s[*pos] = c;
  }
  *pos += 1;
}

static void shim_write_str(char* s, size_t n, size_t* pos, const char* v) {
  if (v == NULL) v = "(null)";
  for (const char* p = v; *p != '\0'; p++) {
    shim_write_char(s, n, pos, *p);
  }
}

static void shim_write_u32_dec(char* s, size_t n, size_t* pos, uint32_t v) {
  char tmp[16];
  size_t len = 0;
  if (v == 0) {
    tmp[len++] = '0';
  } else {
    while (v != 0 && len < sizeof(tmp)) {
      tmp[len++] = (char)('0' + (v % 10u));
      v /= 10u;
    }
  }
  for (size_t i = 0; i < len; i++) {
    shim_write_char(s, n, pos, tmp[len - 1 - i]);
  }
}

static void shim_write_ptr_hex(char* s, size_t n, size_t* pos, uintptr_t v) {
  static const char* digits = "0123456789abcdef";
  char tmp[2 * sizeof(uintptr_t)];
  size_t len = 0;
  if (v == 0) {
    tmp[len++] = '0';
  } else {
    while (v != 0 && len < sizeof(tmp)) {
      tmp[len++] = digits[v & 0xFu];
      v >>= 4u;
    }
  }
  for (size_t i = 0; i < len; i++) {
    shim_write_char(s, n, pos, tmp[len - 1 - i]);
  }
}

int snprintf(char* s, size_t n, const char* fmt, ...) {
  if (s == NULL || n == 0 || fmt == NULL) return 0;

  va_list ap;
  va_start(ap, fmt);

  size_t pos = 0;
  for (const char* p = fmt; *p != '\0'; p++) {
    if (*p != '%') {
      shim_write_char(s, n, &pos, *p);
      continue;
    }

    p++;
    if (*p == '\0') break;
    if (*p == '%') {
      shim_write_char(s, n, &pos, '%');
      continue;
    }
    if (*p == 'u') {
      shim_write_u32_dec(s, n, &pos, va_arg(ap, unsigned int));
      continue;
    }
    if (*p == 'p') {
      shim_write_str(s, n, &pos, "0x");
      shim_write_ptr_hex(s, n, &pos, (uintptr_t)va_arg(ap, void*));
      continue;
    }
    if (*p == 's') {
      shim_write_str(s, n, &pos, va_arg(ap, const char*));
      continue;
    }

    shim_write_char(s, n, &pos, '%');
    shim_write_char(s, n, &pos, *p);
  }

  va_end(ap);

  size_t out_pos = pos;
  if (out_pos >= n) out_pos = n - 1;
  s[out_pos] = '\0';
  return (int)pos;
}

