// x07-wasm Phase 4 native HTTP component glue.
//
// This file is compiled alongside:
// - x07-generated freestanding C output (program.c + x07.h), which exports x07_solve_v2
// - wit-bindgen C bindings for wasi:http/proxy (proxy.c + proxy.h + proxy_component_type.o)
//
// The exported function `exports_wasi_http_incoming_handler_handle` bridges:
//   wasi:http/incoming-handler.handle(request, response-outparam)
// into the Phase 0 ABI:
//   bytes_t x07_solve_v2(uint8_t* arena_mem, uint32_t arena_cap, const uint8_t* input_ptr, uint32_t input_len)
//
// Request/response bytes passed to/from x07 use the pinned JSON envelopes:
// - x07.http.request.envelope@0.1.0
// - x07.http.response.envelope@0.1.0

#include "proxy.h"
#include "x07.h"

#include <stddef.h>
#include <stdint.h>
#include <limits.h>

#ifndef X07_SOLVE_ARENA_CAP_BYTES
#define X07_SOLVE_ARENA_CAP_BYTES (64u * 1024u * 1024u)
#endif

#ifndef X07_SOLVE_MAX_OUTPUT_BYTES
#define X07_SOLVE_MAX_OUTPUT_BYTES (16u * 1024u * 1024u)
#endif

#ifndef X07_NATIVE_HTTP_MAX_REQUEST_BODY_BYTES
#define X07_NATIVE_HTTP_MAX_REQUEST_BODY_BYTES (1024u * 1024u)
#endif

#ifndef X07_NATIVE_HTTP_MAX_RESPONSE_BODY_BYTES
#define X07_NATIVE_HTTP_MAX_RESPONSE_BODY_BYTES (1024u * 1024u)
#endif

#ifndef X07_NATIVE_HTTP_MAX_HEADERS
#define X07_NATIVE_HTTP_MAX_HEADERS (128u)
#endif

#ifndef X07_NATIVE_HTTP_MAX_HEADER_BYTES_TOTAL
#define X07_NATIVE_HTTP_MAX_HEADER_BYTES_TOTAL (64u * 1024u)
#endif

#ifndef X07_NATIVE_HTTP_MAX_PATH_BYTES
#define X07_NATIVE_HTTP_MAX_PATH_BYTES (2048u)
#endif

#ifndef X07_NATIVE_HTTP_MAX_QUERY_BYTES
#define X07_NATIVE_HTTP_MAX_QUERY_BYTES (4096u)
#endif

#ifndef X07_NATIVE_HTTP_MAX_ENVELOPE_BYTES
#define X07_NATIVE_HTTP_MAX_ENVELOPE_BYTES (2u * 1024u * 1024u)
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

static void x07_buf_clear(x07_buf_t *b) {
  b->len = 0;
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

static void x07_buf_push_byte(x07_buf_t *b, uint8_t v) {
  x07_buf_reserve(b, 1u);
  b->ptr[b->len] = v;
  b->len += 1u;
}

static void x07_buf_push_bytes(x07_buf_t *b, const uint8_t *src, size_t n) {
  x07_buf_reserve(b, n);
  for (size_t i = 0; i < n; i++) {
    b->ptr[b->len + i] = src[i];
  }
  b->len += n;
}

static void x07_buf_push_cstr(x07_buf_t *b, const char *s) {
  const uint8_t *p = (const uint8_t *)s;
  while (*p != 0) {
    x07_buf_push_byte(b, *p);
    p++;
  }
}

static void x07_push_u64_dec(x07_buf_t *b, uint64_t n) {
  if (n == 0) {
    x07_buf_push_byte(b, (uint8_t)'0');
    return;
  }
  uint8_t tmp[20];
  size_t i = sizeof(tmp);
  while (n != 0) {
    uint8_t d = (uint8_t)(n % 10u);
    n /= 10u;
    i--;
    tmp[i] = (uint8_t)('0' + d);
  }
  x07_buf_push_bytes(b, &tmp[i], sizeof(tmp) - i);
}

static void x07_push_json_string_bytes_lossy(x07_buf_t *b, const uint8_t *bytes, size_t len) {
  x07_buf_push_byte(b, (uint8_t)'"');
  for (size_t i = 0; i < len; i++) {
    uint8_t ch = bytes[i];
    if (ch == (uint8_t)'"') {
      x07_buf_push_bytes(b, (const uint8_t *)"\\\"", 2u);
    } else if (ch == (uint8_t)'\\') {
      x07_buf_push_bytes(b, (const uint8_t *)"\\\\", 2u);
    } else if (ch <= 0x1fu) {
      static const char hex[] = "0123456789abcdef";
      x07_buf_push_bytes(b, (const uint8_t *)"\\u00", 4u);
      x07_buf_push_byte(b, (uint8_t)hex[(ch >> 4) & 0xfu]);
      x07_buf_push_byte(b, (uint8_t)hex[ch & 0xfu]);
    } else if (ch < 0x80u) {
      x07_buf_push_byte(b, ch);
    } else {
      // Deterministic lossy mapping for non-ASCII bytes.
      x07_buf_push_byte(b, (uint8_t)'?');
    }
  }
  x07_buf_push_byte(b, (uint8_t)'"');
}

static void x07_base64_encode(x07_buf_t *out, const uint8_t *src, size_t len) {
  static const char tbl[] =
      "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  size_t i = 0;
  while (i + 3u <= len) {
    uint32_t v = ((uint32_t)src[i] << 16) | ((uint32_t)src[i + 1u] << 8) |
                 (uint32_t)src[i + 2u];
    x07_buf_push_byte(out, (uint8_t)tbl[(v >> 18) & 63u]);
    x07_buf_push_byte(out, (uint8_t)tbl[(v >> 12) & 63u]);
    x07_buf_push_byte(out, (uint8_t)tbl[(v >> 6) & 63u]);
    x07_buf_push_byte(out, (uint8_t)tbl[v & 63u]);
    i += 3u;
  }
  size_t rem = len - i;
  if (rem == 1u) {
    uint32_t v = ((uint32_t)src[i] << 16);
    x07_buf_push_byte(out, (uint8_t)tbl[(v >> 18) & 63u]);
    x07_buf_push_byte(out, (uint8_t)tbl[(v >> 12) & 63u]);
    x07_buf_push_byte(out, (uint8_t)'=');
    x07_buf_push_byte(out, (uint8_t)'=');
  } else if (rem == 2u) {
    uint32_t v = ((uint32_t)src[i] << 16) | ((uint32_t)src[i + 1u] << 8);
    x07_buf_push_byte(out, (uint8_t)tbl[(v >> 18) & 63u]);
    x07_buf_push_byte(out, (uint8_t)tbl[(v >> 12) & 63u]);
    x07_buf_push_byte(out, (uint8_t)tbl[(v >> 6) & 63u]);
    x07_buf_push_byte(out, (uint8_t)'=');
  }
}

static int x07_cmp_bytes(const uint8_t *a, size_t alen, const uint8_t *b, size_t blen) {
  size_t n = alen < blen ? alen : blen;
  for (size_t i = 0; i < n; i++) {
    if (a[i] < b[i]) return -1;
    if (a[i] > b[i]) return 1;
  }
  if (alen < blen) return -1;
  if (alen > blen) return 1;
  return 0;
}

typedef struct x07_hdr_pair_t {
  const uint8_t *k;
  size_t k_len;
  const uint8_t *v;
  size_t v_len;
} x07_hdr_pair_t;

static int x07_hdr_pair_cmp(const x07_hdr_pair_t *a, const x07_hdr_pair_t *b) {
  int c = x07_cmp_bytes(a->k, a->k_len, b->k, b->k_len);
  if (c != 0) return c;
  return x07_cmp_bytes(a->v, a->v_len, b->v, b->v_len);
}

static void x07_hdr_pairs_sort(x07_hdr_pair_t *arr, size_t n) {
  for (size_t i = 1; i < n; i++) {
    x07_hdr_pair_t key = arr[i];
    size_t j = i;
    while (j > 0) {
      if (x07_hdr_pair_cmp(&arr[j - 1u], &key) <= 0) break;
      arr[j] = arr[j - 1u];
      j--;
    }
    arr[j] = key;
  }
}

static void x07_set_response_outparam_impl(
    exports_wasi_http_incoming_handler_own_response_outparam_t response_out,
    uint16_t status,
    const char *diag_code,
    size_t diag_code_len,
    const char *body_text,
    size_t body_text_len) {
  // Build headers list: diag + content-type.
  proxy_tuple2_field_name_field_value_t h[2];

  static const char k0[] = "x-x07-diag-code";
  h[0].f0.ptr = (uint8_t *)k0;
  h[0].f0.len = sizeof(k0) - 1u;
  h[0].f1.ptr = (uint8_t *)diag_code;
  h[0].f1.len = diag_code_len;

  static const char k1[] = "content-type";
  static const char v1[] = "text/plain; charset=utf-8";
  h[1].f0.ptr = (uint8_t *)k1;
  h[1].f0.len = sizeof(k1) - 1u;
  h[1].f1.ptr = (uint8_t *)v1;
  h[1].f1.len = sizeof(v1) - 1u;

  proxy_list_tuple2_field_name_field_value_t list;
  list.ptr = h;
  list.len = 2u;

  wasi_http_types_own_fields_t fields;
  wasi_http_types_header_error_t herr;
  if (!wasi_http_types_static_fields_from_list(&list, &fields, &herr)) {
    x07_trap();
  }

  wasi_http_types_own_outgoing_response_t resp =
      wasi_http_types_constructor_outgoing_response(fields);
  if (!wasi_http_types_method_outgoing_response_set_status_code(
          wasi_http_types_borrow_outgoing_response(resp), status)) {
    x07_trap();
  }

  wasi_http_types_own_outgoing_body_t out_body;
  if (!wasi_http_types_method_outgoing_response_body(
          wasi_http_types_borrow_outgoing_response(resp), &out_body)) {
    x07_trap();
  }

  wasi_http_types_own_output_stream_t out_stream;
  if (!wasi_http_types_method_outgoing_body_write(
          wasi_http_types_borrow_outgoing_body(out_body), &out_stream)) {
    x07_trap();
  }

  if (body_text != NULL) {
    proxy_list_u8_t chunk;
    chunk.ptr = (uint8_t *)body_text;
    chunk.len = body_text_len;
    wasi_io_streams_stream_error_t werr;
    if (!wasi_io_streams_method_output_stream_blocking_write_and_flush(
            wasi_io_streams_borrow_output_stream(out_stream), &chunk, &werr)) {
      x07_trap();
    }
  }

  wasi_io_streams_output_stream_drop_own(out_stream);

  wasi_http_types_error_code_t berr;
  if (!wasi_http_types_static_outgoing_body_finish(out_body, NULL, &berr)) {
    x07_trap();
  }

  wasi_http_types_result_own_outgoing_response_error_code_t result;
  result.is_err = false;
  result.val.ok = resp;
  wasi_http_types_static_response_outparam_set(
      (wasi_http_types_own_response_outparam_t)response_out, &result);
}

#define x07_set_response_outparam(response_out, status, diag_code, body_text)   \
  x07_set_response_outparam_impl(                                              \
      (response_out),                                                         \
      (status),                                                               \
      (diag_code),                                                            \
      (sizeof(diag_code) - 1u),                                               \
      (body_text),                                                            \
      (sizeof(body_text) - 1u))

static const char *x07_http_method_string(const wasi_http_types_method_t *m,
                                          proxy_string_t *tmp,
                                          size_t *len_out) {
  switch (m->tag) {
  case WASI_HTTP_TYPES_METHOD_GET:
    *len_out = 3u;
    return "GET";
  case WASI_HTTP_TYPES_METHOD_HEAD:
    *len_out = 4u;
    return "HEAD";
  case WASI_HTTP_TYPES_METHOD_POST:
    *len_out = 4u;
    return "POST";
  case WASI_HTTP_TYPES_METHOD_PUT:
    *len_out = 3u;
    return "PUT";
  case WASI_HTTP_TYPES_METHOD_DELETE:
    *len_out = 6u;
    return "DELETE";
  case WASI_HTTP_TYPES_METHOD_CONNECT:
    *len_out = 7u;
    return "CONNECT";
  case WASI_HTTP_TYPES_METHOD_OPTIONS:
    *len_out = 7u;
    return "OPTIONS";
  case WASI_HTTP_TYPES_METHOD_TRACE:
    *len_out = 5u;
    return "TRACE";
  case WASI_HTTP_TYPES_METHOD_PATCH:
    *len_out = 5u;
    return "PATCH";
  case WASI_HTTP_TYPES_METHOD_OTHER:
    *tmp = m->val.other;
    *len_out = 0u;
    return NULL;
  default:
    *len_out = 3u;
    return "GET";
  }
}

typedef struct x07_cur_t {
  const uint8_t *bytes;
  size_t len;
  size_t i;
  int ok;
} x07_cur_t;

static int x07_cur_peek(x07_cur_t *c) {
  if (c->i >= c->len) return -1;
  return (int)c->bytes[c->i];
}

static int x07_cur_next(x07_cur_t *c) {
  int b = x07_cur_peek(c);
  if (b < 0) return -1;
  c->i++;
  return b;
}

static void x07_cur_skip_ws(x07_cur_t *c) {
  for (;;) {
    int b = x07_cur_peek(c);
    if (b == ' ' || b == '\n' || b == '\r' || b == '\t') {
      c->i++;
      continue;
    }
    break;
  }
}

static void x07_cur_expect(x07_cur_t *c, uint8_t want) {
  int b = x07_cur_next(c);
  if (b < 0 || (uint8_t)b != want) c->ok = 0;
}

static uint32_t x07_parse_hex4(x07_cur_t *c) {
  uint32_t v = 0;
  for (int i = 0; i < 4; i++) {
    int b = x07_cur_next(c);
    if (b < 0) {
      c->ok = 0;
      return 0;
    }
    uint8_t ch = (uint8_t)b;
    uint8_t d;
    if (ch >= '0' && ch <= '9')
      d = (uint8_t)(ch - '0');
    else if (ch >= 'a' && ch <= 'f')
      d = (uint8_t)(10u + (ch - 'a'));
    else if (ch >= 'A' && ch <= 'F')
      d = (uint8_t)(10u + (ch - 'A'));
    else {
      c->ok = 0;
      return 0;
    }
    v = (v << 4) | (uint32_t)d;
  }
  return v;
}

static void x07_utf8_push_codepoint(x07_buf_t *b, uint32_t cp) {
  if (cp <= 0x7fu) {
    x07_buf_push_byte(b, (uint8_t)cp);
  } else if (cp <= 0x7ffu) {
    x07_buf_push_byte(b, (uint8_t)(0xc0u | ((cp >> 6) & 0x1fu)));
    x07_buf_push_byte(b, (uint8_t)(0x80u | (cp & 0x3fu)));
  } else if (cp <= 0xffffu) {
    x07_buf_push_byte(b, (uint8_t)(0xe0u | ((cp >> 12) & 0x0fu)));
    x07_buf_push_byte(b, (uint8_t)(0x80u | ((cp >> 6) & 0x3fu)));
    x07_buf_push_byte(b, (uint8_t)(0x80u | (cp & 0x3fu)));
  } else {
    x07_buf_push_byte(b, (uint8_t)(0xf0u | ((cp >> 18) & 0x07u)));
    x07_buf_push_byte(b, (uint8_t)(0x80u | ((cp >> 12) & 0x3fu)));
    x07_buf_push_byte(b, (uint8_t)(0x80u | ((cp >> 6) & 0x3fu)));
    x07_buf_push_byte(b, (uint8_t)(0x80u | (cp & 0x3fu)));
  }
}

static void x07_cur_parse_string(x07_cur_t *c, x07_buf_t *out) {
  x07_buf_clear(out);
  x07_cur_expect(c, (uint8_t)'"');
  while (c->ok) {
    int b = x07_cur_next(c);
    if (b < 0) {
      c->ok = 0;
      break;
    }
    uint8_t ch = (uint8_t)b;
    if (ch == (uint8_t)'"') return;
    if (ch == (uint8_t)'\\') {
      int e = x07_cur_next(c);
      if (e < 0) {
        c->ok = 0;
        break;
      }
      uint8_t esc = (uint8_t)e;
      switch (esc) {
      case '"':
        x07_buf_push_byte(out, (uint8_t)'"');
        break;
      case '\\':
        x07_buf_push_byte(out, (uint8_t)'\\');
        break;
      case '/':
        x07_buf_push_byte(out, (uint8_t)'/');
        break;
      case 'b':
        x07_buf_push_byte(out, 0x08u);
        break;
      case 'f':
        x07_buf_push_byte(out, 0x0cu);
        break;
      case 'n':
        x07_buf_push_byte(out, (uint8_t)'\n');
        break;
      case 'r':
        x07_buf_push_byte(out, (uint8_t)'\r');
        break;
      case 't':
        x07_buf_push_byte(out, (uint8_t)'\t');
        break;
      case 'u': {
        uint32_t a = x07_parse_hex4(c);
        if (!c->ok) break;
        if (a >= 0xd800u && a <= 0xdbffu) {
          size_t saved = c->i;
          if (x07_cur_next(c) != '\\' || x07_cur_next(c) != 'u') {
            c->i = saved;
            c->ok = 0;
            break;
          }
          uint32_t b2 = x07_parse_hex4(c);
          if (!c->ok) break;
          if (!(b2 >= 0xdc00u && b2 <= 0xdfffu)) {
            c->ok = 0;
            break;
          }
          uint32_t hi = a - 0xd800u;
          uint32_t lo = b2 - 0xdc00u;
          uint32_t cp = 0x10000u + (hi << 10) + lo;
          x07_utf8_push_codepoint(out, cp);
        } else {
          x07_utf8_push_codepoint(out, a);
        }
        break;
      }
      default:
        c->ok = 0;
        break;
      }
      continue;
    }
    if (ch <= 0x1fu) {
      c->ok = 0;
      break;
    }
    x07_buf_push_byte(out, ch);
  }
}

static uint64_t x07_cur_parse_u64(x07_cur_t *c) {
  uint64_t n = 0;
  int any = 0;
  while (c->ok) {
    int b = x07_cur_peek(c);
    if (b < '0' || b > '9') break;
    any = 1;
    uint8_t d = (uint8_t)(b - '0');
    if (n > (UINT64_MAX - (uint64_t)d) / 10u) {
      c->ok = 0;
      return 0;
    }
    n = n * 10u + (uint64_t)d;
    c->i++;
  }
  if (!any) c->ok = 0;
  return n;
}

static void x07_cur_expect_bytes(x07_cur_t *c, const char *s) {
  const uint8_t *p = (const uint8_t *)s;
  while (*p != 0 && c->ok) {
    int b = x07_cur_next(c);
    if (b < 0 || (uint8_t)b != *p) {
      c->ok = 0;
      return;
    }
    p++;
  }
}

static void x07_cur_skip_value(x07_cur_t *c);

static void x07_cur_skip_number(x07_cur_t *c) {
  if (x07_cur_peek(c) == '-') c->i++;
  int any = 0;
  while (c->ok) {
    int b = x07_cur_peek(c);
    if (b < '0' || b > '9') break;
    any = 1;
    c->i++;
  }
  if (!any) c->ok = 0;
  if (x07_cur_peek(c) == '.') {
    c->i++;
    int any_frac = 0;
    while (c->ok) {
      int b = x07_cur_peek(c);
      if (b < '0' || b > '9') break;
      any_frac = 1;
      c->i++;
    }
    if (!any_frac) c->ok = 0;
  }
  int e = x07_cur_peek(c);
  if (e == 'e' || e == 'E') {
    c->i++;
    int s = x07_cur_peek(c);
    if (s == '+' || s == '-') c->i++;
    int any_exp = 0;
    while (c->ok) {
      int b = x07_cur_peek(c);
      if (b < '0' || b > '9') break;
      any_exp = 1;
      c->i++;
    }
    if (!any_exp) c->ok = 0;
  }
}

static void x07_cur_skip_array(x07_cur_t *c) {
  x07_cur_expect(c, (uint8_t)'[');
  x07_cur_skip_ws(c);
  if (x07_cur_peek(c) == ']') {
    c->i++;
    return;
  }
  for (;;) {
    x07_cur_skip_ws(c);
    x07_cur_skip_value(c);
    x07_cur_skip_ws(c);
    int b = x07_cur_next(c);
    if (b == ',') continue;
    if (b == ']') break;
    c->ok = 0;
    break;
  }
}

static void x07_cur_skip_object(x07_cur_t *c) {
  x07_cur_expect(c, (uint8_t)'{');
  x07_cur_skip_ws(c);
  if (x07_cur_peek(c) == '}') {
    c->i++;
    return;
  }
  x07_buf_t tmp;
  x07_buf_init(&tmp);
  for (;;) {
    x07_cur_skip_ws(c);
    x07_cur_parse_string(c, &tmp);
    x07_cur_skip_ws(c);
    x07_cur_expect(c, (uint8_t)':');
    x07_cur_skip_ws(c);
    x07_cur_skip_value(c);
    x07_cur_skip_ws(c);
    int b = x07_cur_next(c);
    if (b == ',') continue;
    if (b == '}') break;
    c->ok = 0;
    break;
  }
}

static void x07_cur_skip_value(x07_cur_t *c) {
  x07_cur_skip_ws(c);
  int b = x07_cur_peek(c);
  if (b == '{') {
    x07_cur_skip_object(c);
    return;
  }
  if (b == '[') {
    x07_cur_skip_array(c);
    return;
  }
  if (b == '"') {
    x07_buf_t tmp;
    x07_buf_init(&tmp);
    x07_cur_parse_string(c, &tmp);
    return;
  }
  if (b == '-' || (b >= '0' && b <= '9')) {
    x07_cur_skip_number(c);
    return;
  }
  if (b == 't') {
    x07_cur_expect_bytes(c, "true");
    return;
  }
  if (b == 'f') {
    x07_cur_expect_bytes(c, "false");
    return;
  }
  if (b == 'n') {
    x07_cur_expect_bytes(c, "null");
    return;
  }
  c->ok = 0;
}

static int x07_base64_value(uint8_t ch) {
  if (ch >= 'A' && ch <= 'Z') return (int)(ch - 'A');
  if (ch >= 'a' && ch <= 'z') return (int)(26 + (ch - 'a'));
  if (ch >= '0' && ch <= '9') return (int)(52 + (ch - '0'));
  if (ch == '+') return 62;
  if (ch == '/') return 63;
  return -1;
}

static int x07_base64_decode_strict(const uint8_t *src, size_t len, x07_buf_t *out) {
  if (len % 4u != 0) return 0;
  x07_buf_clear(out);
  for (size_t i = 0; i < len; i += 4u) {
    uint8_t c0 = src[i];
    uint8_t c1 = src[i + 1u];
    uint8_t c2 = src[i + 2u];
    uint8_t c3 = src[i + 3u];

    int v0 = x07_base64_value(c0);
    int v1 = x07_base64_value(c1);
    if (v0 < 0 || v1 < 0) return 0;

    int pad2 = (c2 == '=');
    int pad3 = (c3 == '=');
    int v2 = pad2 ? 0 : x07_base64_value(c2);
    int v3 = pad3 ? 0 : x07_base64_value(c3);
    if (!pad2 && v2 < 0) return 0;
    if (!pad3 && v3 < 0) return 0;
    if (pad2 && !pad3) return 0;

    uint32_t v = ((uint32_t)v0 << 18) | ((uint32_t)v1 << 12) |
                 ((uint32_t)v2 << 6) | (uint32_t)v3;
    x07_buf_push_byte(out, (uint8_t)((v >> 16) & 0xffu));
    if (!pad2) x07_buf_push_byte(out, (uint8_t)((v >> 8) & 0xffu));
    if (!pad3) x07_buf_push_byte(out, (uint8_t)(v & 0xffu));
  }
  return 1;
}

typedef struct x07_http_resp_env_t {
  uint16_t status;
  x07_hdr_pair_t *headers;
  size_t headers_len;
  uint8_t *body;
  size_t body_len;
} x07_http_resp_env_t;

static void x07_http_resp_env_init(x07_http_resp_env_t *e) {
  e->status = 500u;
  e->headers = NULL;
  e->headers_len = 0;
  e->body = NULL;
  e->body_len = 0;
}

static void x07_http_resp_env_free(x07_http_resp_env_t *e) {
  (void)e;
}

static uint8_t *x07_alloc_copy_bytes(const uint8_t *src, size_t len) {
  if (len == 0) return NULL;
  uint8_t *p = (uint8_t *)cabi_realloc(NULL, 0, 1, len);
  if (p == NULL) x07_trap();
  for (size_t i = 0; i < len; i++) p[i] = src[i];
  return p;
}

static int x07_parse_response_envelope(
    const uint8_t *bytes,
    size_t len,
    x07_http_resp_env_t *out,
    uint32_t *is_json_ok) {
  *is_json_ok = 0;

  x07_cur_t c;
  c.bytes = bytes;
  c.len = len;
  c.i = 0;
  c.ok = 1;

  x07_buf_t key;
  x07_buf_t tmp;
  x07_buf_t b64;
  x07_buf_t decoded;
  x07_buf_init(&key);
  x07_buf_init(&tmp);
  x07_buf_init(&b64);
  x07_buf_init(&decoded);

  int have_schema_version = 0;
  int have_request_id = 0;
  int have_status = 0;
  int have_headers = 0;
  int have_body = 0;

  int invalid = 0;

  x07_cur_skip_ws(&c);
  x07_cur_expect(&c, (uint8_t)'{');
  if (!c.ok) return 0;

  x07_cur_skip_ws(&c);
  if (x07_cur_peek(&c) == '}') {
    c.i++;
    *is_json_ok = 1;
    return 0;
  }

  for (;;) {
    x07_cur_skip_ws(&c);
    if (x07_cur_peek(&c) != '"') {
      c.ok = 0;
      break;
    }
    x07_cur_parse_string(&c, &key);
    if (!c.ok) break;
    x07_cur_skip_ws(&c);
    x07_cur_expect(&c, (uint8_t)':');
    if (!c.ok) break;
    x07_cur_skip_ws(&c);

    // Compare keys using bytes.
    const uint8_t *k = key.ptr;
    size_t klen = key.len;

    // schema_version
    if (klen == 14u &&
        x07_cmp_bytes(k, klen, (const uint8_t *)"schema_version", 14u) == 0) {
      if (x07_cur_peek(&c) != '"') {
        invalid = 1;
        x07_cur_skip_value(&c);
      } else {
        x07_cur_parse_string(&c, &tmp);
        if (!c.ok) break;
        have_schema_version = 1;
        if (tmp.len != 32u ||
            x07_cmp_bytes(tmp.ptr, tmp.len,
                          (const uint8_t *)"x07.http.response.envelope@0.1.0",
                          32u) != 0) {
          invalid = 1;
        }
      }
    }
    // request_id
    else if (klen == 10u &&
             x07_cmp_bytes(k, klen, (const uint8_t *)"request_id", 10u) ==
                 0) {
      if (x07_cur_peek(&c) != '"') {
        invalid = 1;
        x07_cur_skip_value(&c);
      } else {
        x07_cur_parse_string(&c, &tmp);
        if (!c.ok) break;
        have_request_id = 1;
        // value ignored in Phase 4 glue; request_id is a consistency check only.
      }
    }
    // status
    else if (klen == 6u &&
             x07_cmp_bytes(k, klen, (const uint8_t *)"status", 6u) == 0) {
      int b = x07_cur_peek(&c);
      if (!(b >= '0' && b <= '9')) {
        invalid = 1;
        x07_cur_skip_value(&c);
      } else {
        uint64_t n = x07_cur_parse_u64(&c);
        if (!c.ok) break;
        have_status = 1;
        if (n > 65535u) invalid = 1;
        out->status = (uint16_t)n;
      }
    }
    // headers
    else if (klen == 7u &&
             x07_cmp_bytes(k, klen, (const uint8_t *)"headers", 7u) == 0) {
      if (x07_cur_peek(&c) != '[') {
        invalid = 1;
        x07_cur_skip_value(&c);
      } else {
        x07_cur_expect(&c, (uint8_t)'[');
        if (!c.ok) break;
        x07_cur_skip_ws(&c);
        // Collect headers into a temporary array.
        x07_hdr_pair_t *hdrs = NULL;
        size_t hdrs_len = 0;
        size_t hdrs_cap = 0;
        if (x07_cur_peek(&c) != ']') {
          for (;;) {
            x07_cur_skip_ws(&c);
            if (x07_cur_peek(&c) != '{') {
              c.ok = 0;
              break;
            }
            x07_cur_expect(&c, (uint8_t)'{');
            if (!c.ok) break;

            int have_k = 0;
            int have_v = 0;
            x07_buf_t hk;
            x07_buf_t hv;
            x07_buf_init(&hk);
            x07_buf_init(&hv);

            x07_cur_skip_ws(&c);
            if (x07_cur_peek(&c) == '}') {
              invalid = 1;
              c.ok = 0;
              break;
            }
            for (;;) {
              x07_cur_skip_ws(&c);
              if (x07_cur_peek(&c) != '"') {
                c.ok = 0;
                break;
              }
              x07_cur_parse_string(&c, &key);
              if (!c.ok) break;
              x07_cur_skip_ws(&c);
              x07_cur_expect(&c, (uint8_t)':');
              if (!c.ok) break;
              x07_cur_skip_ws(&c);

              if (key.len == 1u && key.ptr[0] == 'k') {
                if (x07_cur_peek(&c) != '"') {
                  invalid = 1;
                  x07_cur_skip_value(&c);
                } else {
                  x07_cur_parse_string(&c, &hk);
                  if (!c.ok) break;
                  have_k = 1;
                }
              } else if (key.len == 1u && key.ptr[0] == 'v') {
                if (x07_cur_peek(&c) != '"') {
                  invalid = 1;
                  x07_cur_skip_value(&c);
                } else {
                  x07_cur_parse_string(&c, &hv);
                  if (!c.ok) break;
                  have_v = 1;
                }
              } else {
                x07_cur_skip_value(&c);
              }

              x07_cur_skip_ws(&c);
              int sep = x07_cur_next(&c);
              if (sep == ',') continue;
              if (sep == '}') break;
              c.ok = 0;
              break;
            }
            if (!c.ok) break;
            if (!have_k || !have_v) invalid = 1;

            // Ensure capacity.
            if (hdrs_len == hdrs_cap) {
              size_t new_cap = hdrs_cap ? hdrs_cap * 2u : 8u;
              x07_hdr_pair_t *p = (x07_hdr_pair_t *)cabi_realloc(
                  hdrs, hdrs_cap * sizeof(x07_hdr_pair_t), 8, new_cap * sizeof(x07_hdr_pair_t));
              if (p == NULL) x07_trap();
              hdrs = p;
              hdrs_cap = new_cap;
            }
            x07_hdr_pair_t pair;
            pair.k = hk.ptr;
            pair.k_len = hk.len;
            pair.v = hv.ptr;
            pair.v_len = hv.len;
            hdrs[hdrs_len++] = pair;

            x07_cur_skip_ws(&c);
            int sep2 = x07_cur_next(&c);
            if (sep2 == ',') continue;
            if (sep2 == ']') break;
            c.ok = 0;
            break;
          }
        } else {
          x07_cur_next(&c); // ]
        }
        if (!c.ok) break;

        have_headers = 1;
        out->headers = hdrs;
        out->headers_len = hdrs_len;
      }
    }
    // body
    else if (klen == 4u &&
             x07_cmp_bytes(k, klen, (const uint8_t *)"body", 4u) == 0) {
      if (x07_cur_peek(&c) != '{') {
        invalid = 1;
        x07_cur_skip_value(&c);
      } else {
        x07_cur_expect(&c, (uint8_t)'{');
        if (!c.ok) break;

        uint64_t bytes_len = (uint64_t)-1;
        int have_bytes_len = 0;
        int have_b64 = 0;
        int have_text = 0;

        x07_cur_skip_ws(&c);
        if (x07_cur_peek(&c) == '}') {
          invalid = 1;
          c.ok = 0;
          break;
        }
        for (;;) {
          x07_cur_skip_ws(&c);
          if (x07_cur_peek(&c) != '"') {
            c.ok = 0;
            break;
          }
          x07_cur_parse_string(&c, &key);
          if (!c.ok) break;
          x07_cur_skip_ws(&c);
          x07_cur_expect(&c, (uint8_t)':');
          if (!c.ok) break;
          x07_cur_skip_ws(&c);

          if (key.len == 9u &&
              x07_cmp_bytes(key.ptr, key.len, (const uint8_t *)"bytes_len", 9u) == 0) {
            int b = x07_cur_peek(&c);
            if (!(b >= '0' && b <= '9')) {
              invalid = 1;
              x07_cur_skip_value(&c);
            } else {
              bytes_len = x07_cur_parse_u64(&c);
              if (!c.ok) break;
              have_bytes_len = 1;
            }
          } else if (key.len == 6u &&
                     x07_cmp_bytes(key.ptr, key.len, (const uint8_t *)"base64", 6u) == 0) {
            if (x07_cur_peek(&c) != '"') {
              invalid = 1;
              x07_cur_skip_value(&c);
            } else {
              x07_cur_parse_string(&c, &b64);
              if (!c.ok) break;
              have_b64 = 1;
            }
          } else if (key.len == 4u &&
                     x07_cmp_bytes(key.ptr, key.len, (const uint8_t *)"text", 4u) == 0) {
            if (x07_cur_peek(&c) != '"') {
              invalid = 1;
              x07_cur_skip_value(&c);
            } else {
              x07_cur_parse_string(&c, &tmp);
              if (!c.ok) break;
              have_text = 1;
            }
          } else {
            x07_cur_skip_value(&c);
          }

          x07_cur_skip_ws(&c);
          int sep = x07_cur_next(&c);
          if (sep == ',') continue;
          if (sep == '}') break;
          c.ok = 0;
          break;
        }
        if (!c.ok) break;

        if (!have_bytes_len) invalid = 1;
        if (bytes_len > 0xffffffffu) invalid = 1;

        if (bytes_len > (uint64_t)X07_NATIVE_HTTP_MAX_RESPONSE_BODY_BYTES) {
          // The envelope is syntactically valid; treat as a budget error later.
          out->body = NULL;
          out->body_len = (size_t)bytes_len;
        } else if (have_b64) {
          if (!x07_base64_decode_strict(b64.ptr, b64.len, &decoded)) {
            *is_json_ok = 1;
            return -2; // b64 invalid
          }
          if ((uint64_t)decoded.len != bytes_len) invalid = 1;
          out->body = decoded.ptr;
          out->body_len = decoded.len;
        } else if (have_text) {
          if ((uint64_t)tmp.len != bytes_len) invalid = 1;
          out->body = x07_alloc_copy_bytes(tmp.ptr, tmp.len);
          out->body_len = tmp.len;
        } else if (bytes_len == 0) {
          out->body = NULL;
          out->body_len = 0;
        } else {
          invalid = 1;
        }

        have_body = 1;
      }
    } else {
      x07_cur_skip_value(&c);
    }

    x07_cur_skip_ws(&c);
    int sep = x07_cur_next(&c);
    if (sep == ',') continue;
    if (sep == '}') break;
    c.ok = 0;
    break;
  }

  if (!c.ok) return 0;
  *is_json_ok = 1;

  if (!have_schema_version || !have_request_id || !have_status || !have_headers || !have_body)
    invalid = 1;

  if (invalid) return -1;
  return 1;
}

void exports_wasi_http_incoming_handler_handle(
    exports_wasi_http_incoming_handler_own_incoming_request_t request,
    exports_wasi_http_incoming_handler_own_response_outparam_t response_out) {
  // Allocate the x07 arena outside the per-call bump reset region.
  if (x07_arena == NULL) {
    x07_arena =
        (uint8_t *)cabi_realloc(NULL, 0, 8, (size_t)X07_SOLVE_ARENA_CAP_BYTES);
    if (x07_arena == NULL) x07_trap();
  }

  uintptr_t call_mark = x07_heap_ptr;

  wasi_http_types_borrow_incoming_request_t req_b =
      wasi_http_types_borrow_incoming_request((wasi_http_types_own_incoming_request_t)request);

  // Method.
  wasi_http_types_method_t method;
  wasi_http_types_method_incoming_request_method(req_b, &method);
  proxy_string_t method_other;
  size_t method_const_len = 0u;
  const char *method_const =
      x07_http_method_string(&method, &method_other, &method_const_len);

  // Path + query.
  proxy_string_t path_with_query;
  const uint8_t *path_ptr = (const uint8_t *)"/";
  size_t path_len = 1u;
  const uint8_t *query_ptr = (const uint8_t *)"";
  size_t query_len = 0u;
  if (wasi_http_types_method_incoming_request_path_with_query(req_b, &path_with_query)) {
    const uint8_t *s = path_with_query.ptr;
    size_t slen = path_with_query.len;
    size_t qpos = (size_t)-1;
    for (size_t i = 0; i < slen; i++) {
      if (s[i] == (uint8_t)'?') {
        qpos = i;
        break;
      }
    }
    if (qpos == (size_t)-1) {
      path_ptr = s;
      path_len = slen;
      query_ptr = (const uint8_t *)"";
      query_len = 0u;
    } else {
      path_ptr = s;
      path_len = qpos;
      query_ptr = s + qpos + 1u;
      query_len = slen - qpos - 1u;
    }
  }

  if ((uint64_t)path_len > (uint64_t)X07_NATIVE_HTTP_MAX_PATH_BYTES) {
    x07_set_response_outparam(
        response_out, 413u, "X07WASM_BUDGET_EXCEEDED_HTTP_PATH_BYTES", "path too large");
    wasi_http_types_incoming_request_drop_own(
        (wasi_http_types_own_incoming_request_t)request);
    x07_heap_ptr = call_mark;
    return;
  }
  if ((uint64_t)query_len > (uint64_t)X07_NATIVE_HTTP_MAX_QUERY_BYTES) {
    x07_set_response_outparam(
        response_out, 413u, "X07WASM_BUDGET_EXCEEDED_HTTP_QUERY_BYTES", "query too large");
    wasi_http_types_incoming_request_drop_own(
        (wasi_http_types_own_incoming_request_t)request);
    x07_heap_ptr = call_mark;
    return;
  }

  // Headers.
  wasi_http_types_own_headers_t headers_res =
      wasi_http_types_method_incoming_request_headers(req_b);
  proxy_list_tuple2_field_name_field_value_t hdr_entries;
  wasi_http_types_method_fields_entries(
      wasi_http_types_borrow_fields(headers_res), &hdr_entries);
  wasi_http_types_fields_drop_own(headers_res);

  if ((uint64_t)hdr_entries.len > (uint64_t)X07_NATIVE_HTTP_MAX_HEADERS) {
    x07_set_response_outparam(
        response_out, 413u, "X07WASM_BUDGET_EXCEEDED_HTTP_HEADERS", "too many headers");
    wasi_http_types_incoming_request_drop_own(
        (wasi_http_types_own_incoming_request_t)request);
    x07_heap_ptr = call_mark;
    return;
  }

  uint64_t header_bytes_total = 0;
  x07_hdr_pair_t *hdrs = NULL;
  if (hdr_entries.len > 0) {
    hdrs = (x07_hdr_pair_t *)cabi_realloc(
        NULL, 0, 8, hdr_entries.len * sizeof(x07_hdr_pair_t));
    if (hdrs == NULL) x07_trap();
    for (size_t i = 0; i < hdr_entries.len; i++) {
      proxy_tuple2_field_name_field_value_t t = hdr_entries.ptr[i];
      hdrs[i].k = t.f0.ptr;
      hdrs[i].k_len = t.f0.len;
      hdrs[i].v = t.f1.ptr;
      hdrs[i].v_len = t.f1.len;
      header_bytes_total += (uint64_t)t.f0.len + (uint64_t)t.f1.len;
    }
    x07_hdr_pairs_sort(hdrs, hdr_entries.len);
  }

  if (header_bytes_total > (uint64_t)X07_NATIVE_HTTP_MAX_HEADER_BYTES_TOTAL) {
    x07_set_response_outparam(
        response_out,
        413u,
        "X07WASM_BUDGET_EXCEEDED_HTTP_HEADER_BYTES_TOTAL",
        "headers too large");
    wasi_http_types_incoming_request_drop_own(
        (wasi_http_types_own_incoming_request_t)request);
    x07_heap_ptr = call_mark;
    return;
  }

  // Body (bounded).
  x07_buf_t body;
  x07_buf_init(&body);
  uint64_t body_len = 0;
  wasi_http_types_own_incoming_body_t incoming_body;
  if (wasi_http_types_method_incoming_request_consume(req_b, &incoming_body)) {
    wasi_http_types_own_input_stream_t in_stream;
    if (wasi_http_types_method_incoming_body_stream(
            wasi_http_types_borrow_incoming_body(incoming_body), &in_stream)) {
      wasi_io_streams_borrow_input_stream_t in_b =
          wasi_io_streams_borrow_input_stream(in_stream);
      for (;;) {
        proxy_list_u8_t chunk;
        wasi_io_streams_stream_error_t err;
        if (!wasi_io_streams_method_input_stream_blocking_read(
                in_b, 4096u, &chunk, &err)) {
          if (err.tag == WASI_IO_STREAMS_STREAM_ERROR_CLOSED) break;
          wasi_io_streams_input_stream_drop_own(in_stream);
          wasi_http_types_own_future_trailers_t ft =
              wasi_http_types_static_incoming_body_finish(incoming_body);
          wasi_http_types_future_trailers_drop_own(ft);
          // Read failure: treat as request failure.
          x07_set_response_outparam(
              response_out,
              500u,
              "X07WASM_SERVE_REQUEST_FAILED",
              "incoming body read failed");
          wasi_http_types_incoming_request_drop_own(
              (wasi_http_types_own_incoming_request_t)request);
          x07_heap_ptr = call_mark;
          return;
        }
        if (chunk.len == 0) {
          proxy_list_u8_free(&chunk);
          break;
        }
        if (body_len + (uint64_t)chunk.len >
            (uint64_t)X07_NATIVE_HTTP_MAX_REQUEST_BODY_BYTES) {
          proxy_list_u8_free(&chunk);
          // Must drop the input stream before finishing the incoming body.
          wasi_io_streams_input_stream_drop_own(in_stream);
          wasi_http_types_own_future_trailers_t ft =
              wasi_http_types_static_incoming_body_finish(incoming_body);
          wasi_http_types_future_trailers_drop_own(ft);
          x07_set_response_outparam(
              response_out,
              413u,
              "X07WASM_BUDGET_EXCEEDED_HTTP_REQUEST_BODY",
              "request body too large");
          wasi_http_types_incoming_request_drop_own(
              (wasi_http_types_own_incoming_request_t)request);
          x07_heap_ptr = call_mark;
          return;
        }
        x07_buf_push_bytes(&body, chunk.ptr, chunk.len);
        body_len += (uint64_t)chunk.len;
        proxy_list_u8_free(&chunk);
      }
      wasi_io_streams_input_stream_drop_own(in_stream);
    }
    wasi_http_types_own_future_trailers_t ft =
        wasi_http_types_static_incoming_body_finish(incoming_body);
    wasi_http_types_future_trailers_drop_own(ft);
  }

  // Base64 encode request body.
  x07_buf_t body_b64;
  x07_buf_init(&body_b64);
  if (body.len > 0) {
    x07_base64_encode(&body_b64, body.ptr, body.len);
  }

  // Build request envelope JSON.
  x07_buf_t env;
  x07_buf_init(&env);
  x07_buf_push_cstr(&env, "{\"schema_version\":\"x07.http.request.envelope@0.1.0\",\"id\":\"req0\",\"method\":");
  if (method_const != NULL) {
    x07_push_json_string_bytes_lossy(
        &env, (const uint8_t *)method_const, method_const_len);
  } else {
    x07_push_json_string_bytes_lossy(&env, method_other.ptr, method_other.len);
  }
  x07_buf_push_cstr(&env, ",\"path\":");
  x07_push_json_string_bytes_lossy(&env, path_ptr, path_len);
  x07_buf_push_cstr(&env, ",\"query\":");
  x07_push_json_string_bytes_lossy(&env, query_ptr, query_len);
  x07_buf_push_cstr(&env, ",\"headers\":[");
  for (size_t i = 0; i < hdr_entries.len; i++) {
    if (i != 0) x07_buf_push_byte(&env, (uint8_t)',');
    x07_buf_push_cstr(&env, "{\"k\":");
    x07_push_json_string_bytes_lossy(&env, hdrs[i].k, hdrs[i].k_len);
    x07_buf_push_cstr(&env, ",\"v\":");
    x07_push_json_string_bytes_lossy(&env, hdrs[i].v, hdrs[i].v_len);
    x07_buf_push_byte(&env, (uint8_t)'}');
  }
  x07_buf_push_cstr(&env, "],\"body\":{\"bytes_len\":");
  x07_push_u64_dec(&env, (uint64_t)body.len);
  x07_buf_push_cstr(&env, ",\"base64\":");
  x07_push_json_string_bytes_lossy(&env, body_b64.ptr, body_b64.len);
  x07_buf_push_bytes(&env, (const uint8_t *)"}}", 2u);

  if ((uint64_t)env.len > (uint64_t)X07_NATIVE_HTTP_MAX_ENVELOPE_BYTES) {
    x07_set_response_outparam(
        response_out,
        413u,
        "X07WASM_BUDGET_EXCEEDED_HTTP_ENVELOPE_BYTES",
        "envelope too large");
    wasi_http_types_incoming_request_drop_own(
        (wasi_http_types_own_incoming_request_t)request);
    x07_heap_ptr = call_mark;
    return;
  }

  if (env.len > 0xffffffffu) x07_trap();

  bytes_t out = x07_solve_v2(
      x07_arena,
      (uint32_t)X07_SOLVE_ARENA_CAP_BYTES,
      (const uint8_t *)env.ptr,
      (uint32_t)env.len);

  if (out.len > (uint32_t)X07_SOLVE_MAX_OUTPUT_BYTES) x07_trap();

  uintptr_t out_ptr = (uintptr_t)out.ptr;
  uintptr_t out_end = out_ptr + (uintptr_t)out.len;
  uintptr_t arena_ptr = (uintptr_t)x07_arena;
  uintptr_t arena_end = arena_ptr + (uintptr_t)X07_SOLVE_ARENA_CAP_BYTES;
  if (out_ptr < arena_ptr || out_end > arena_end) x07_trap();

  x07_http_resp_env_t resp_env;
  x07_http_resp_env_init(&resp_env);

  uint32_t json_ok = 0;
  int parsed = x07_parse_response_envelope(
      (const uint8_t *)out.ptr, (size_t)out.len, &resp_env, &json_ok);

  if (!json_ok) {
    x07_set_response_outparam(
        response_out,
        500u,
        "X07WASM_NATIVE_HTTP_RESPONSE_ENVELOPE_PARSE_FAILED",
        "response envelope parse failed");
    wasi_http_types_incoming_request_drop_own(
        (wasi_http_types_own_incoming_request_t)request);
    x07_heap_ptr = call_mark;
    return;
  }
  if (parsed == -2) {
    x07_set_response_outparam(
        response_out,
        500u,
        "X07WASM_NATIVE_HTTP_RESPONSE_BODY_B64_INVALID",
        "response body base64 invalid");
    wasi_http_types_incoming_request_drop_own(
        (wasi_http_types_own_incoming_request_t)request);
    x07_heap_ptr = call_mark;
    return;
  }
  if (parsed != 1) {
    x07_set_response_outparam(
        response_out,
        500u,
        "X07WASM_NATIVE_HTTP_RESPONSE_ENVELOPE_INVALID",
        "response envelope invalid");
    wasi_http_types_incoming_request_drop_own(
        (wasi_http_types_own_incoming_request_t)request);
    x07_heap_ptr = call_mark;
    return;
  }

  // Response budgets.
  if ((uint64_t)resp_env.headers_len > (uint64_t)X07_NATIVE_HTTP_MAX_HEADERS) {
    x07_set_response_outparam(
        response_out,
        500u,
        "X07WASM_BUDGET_EXCEEDED_HTTP_HEADERS",
        "response headers too many");
    wasi_http_types_incoming_request_drop_own(
        (wasi_http_types_own_incoming_request_t)request);
    x07_heap_ptr = call_mark;
    return;
  }

  uint64_t resp_header_bytes_total = 0;
  for (size_t i = 0; i < resp_env.headers_len; i++) {
    resp_header_bytes_total += (uint64_t)resp_env.headers[i].k_len +
                               (uint64_t)resp_env.headers[i].v_len;
  }
  if (resp_header_bytes_total > (uint64_t)X07_NATIVE_HTTP_MAX_HEADER_BYTES_TOTAL) {
    x07_set_response_outparam(
        response_out,
        500u,
        "X07WASM_BUDGET_EXCEEDED_HTTP_HEADER_BYTES_TOTAL",
        "response headers too large");
    wasi_http_types_incoming_request_drop_own(
        (wasi_http_types_own_incoming_request_t)request);
    x07_heap_ptr = call_mark;
    return;
  }

  if ((uint64_t)resp_env.body_len > (uint64_t)X07_NATIVE_HTTP_MAX_RESPONSE_BODY_BYTES) {
    x07_set_response_outparam(
        response_out,
        500u,
        "X07WASM_BUDGET_EXCEEDED_HTTP_RESPONSE_BODY",
        "response body too large");
    wasi_http_types_incoming_request_drop_own(
        (wasi_http_types_own_incoming_request_t)request);
    x07_heap_ptr = call_mark;
    return;
  }

  // Build outgoing headers.
  proxy_list_tuple2_field_name_field_value_t resp_list;
  resp_list.len = resp_env.headers_len;
  proxy_tuple2_field_name_field_value_t *resp_pairs = NULL;
  if (resp_env.headers_len > 0) {
    resp_pairs = (proxy_tuple2_field_name_field_value_t *)cabi_realloc(
        NULL, 0, 8, resp_env.headers_len * sizeof(proxy_tuple2_field_name_field_value_t));
    if (resp_pairs == NULL) x07_trap();
    for (size_t i = 0; i < resp_env.headers_len; i++) {
      resp_pairs[i].f0.ptr = (uint8_t *)resp_env.headers[i].k;
      resp_pairs[i].f0.len = resp_env.headers[i].k_len;
      resp_pairs[i].f1.ptr = (uint8_t *)resp_env.headers[i].v;
      resp_pairs[i].f1.len = resp_env.headers[i].v_len;
    }
  }
  resp_list.ptr = resp_pairs;

  wasi_http_types_own_fields_t resp_fields;
  wasi_http_types_header_error_t herr;
  if (!wasi_http_types_static_fields_from_list(&resp_list, &resp_fields, &herr)) {
    x07_set_response_outparam(
        response_out,
        500u,
        "X07WASM_NATIVE_HTTP_RESPONSE_ENVELOPE_INVALID",
        "response headers invalid for wasi:http");
    wasi_http_types_incoming_request_drop_own(
        (wasi_http_types_own_incoming_request_t)request);
    x07_heap_ptr = call_mark;
    return;
  }

  wasi_http_types_own_outgoing_response_t out_resp =
      wasi_http_types_constructor_outgoing_response(resp_fields);
  if (!wasi_http_types_method_outgoing_response_set_status_code(
          wasi_http_types_borrow_outgoing_response(out_resp), resp_env.status)) {
    x07_set_response_outparam(
        response_out,
        500u,
        "X07WASM_NATIVE_HTTP_RESPONSE_ENVELOPE_INVALID",
        "invalid status code");
    wasi_http_types_incoming_request_drop_own(
        (wasi_http_types_own_incoming_request_t)request);
    x07_heap_ptr = call_mark;
    return;
  }

  wasi_http_types_own_outgoing_body_t out_body;
  if (!wasi_http_types_method_outgoing_response_body(
          wasi_http_types_borrow_outgoing_response(out_resp), &out_body)) {
    x07_trap();
  }
  wasi_http_types_own_output_stream_t out_stream;
  if (!wasi_http_types_method_outgoing_body_write(
          wasi_http_types_borrow_outgoing_body(out_body), &out_stream)) {
    x07_trap();
  }

  if (resp_env.body_len > 0) {
    const uint8_t *p = resp_env.body;
    size_t remaining = resp_env.body_len;
    while (remaining > 0) {
      size_t n = remaining < 4096u ? remaining : 4096u;
      proxy_list_u8_t chunk;
      chunk.ptr = (uint8_t *)p;
      chunk.len = n;
      wasi_io_streams_stream_error_t werr;
      if (!wasi_io_streams_method_output_stream_blocking_write_and_flush(
              wasi_io_streams_borrow_output_stream(out_stream), &chunk, &werr)) {
        x07_trap();
      }
      p += n;
      remaining -= n;
    }
  }

  wasi_io_streams_output_stream_drop_own(out_stream);

  wasi_http_types_error_code_t berr;
  if (!wasi_http_types_static_outgoing_body_finish(out_body, NULL, &berr)) {
    x07_trap();
  }

  wasi_http_types_result_own_outgoing_response_error_code_t result;
  result.is_err = false;
  result.val.ok = out_resp;
  wasi_http_types_static_response_outparam_set(
      (wasi_http_types_own_response_outparam_t)response_out, &result);

  x07_http_resp_env_free(&resp_env);
  wasi_http_types_incoming_request_drop_own(
      (wasi_http_types_own_incoming_request_t)request);
  x07_heap_ptr = call_mark;
}
