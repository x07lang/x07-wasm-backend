#include "x07.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static uint8_t* read_file(const char* path, size_t* out_len) {
  FILE* f = fopen(path, "rb");
  if (!f) return NULL;

  if (fseek(f, 0, SEEK_END) != 0) { fclose(f); return NULL; }
  long n = ftell(f);
  if (n < 0) { fclose(f); return NULL; }
  if (fseek(f, 0, SEEK_SET) != 0) { fclose(f); return NULL; }

  size_t len = (size_t)n;
  uint8_t* buf = (uint8_t*)malloc(len ? len : 1);
  if (!buf) { fclose(f); return NULL; }

  if (len) {
    size_t got = fread(buf, 1, len, f);
    if (got != len) { fclose(f); free(buf); return NULL; }
  }

  fclose(f);
  *out_len = len;
  return buf;
}

static int bytes_eq(const uint8_t* a, size_t a_len, const uint8_t* b, size_t b_len) {
  if (a_len != b_len) return 0;
  for (size_t i = 0; i < a_len; i++) {
    if (a[i] != b[i]) return 0;
  }
  return 1;
}

int main(int argc, char** argv) {
  if (argc != 3) {
    fprintf(stderr, "usage: %s <input.bin> <expected.bin>\n", argv[0]);
    return 2;
  }

  const char* in_path = argv[1];
  const char* exp_path = argv[2];

  size_t in_len = 0;
  uint8_t* in_buf = read_file(in_path, &in_len);
  if (!in_buf && in_len == 0) {
    fprintf(stderr, "failed to read input: %s\n", in_path);
    return 2;
  }

  size_t exp_len = 0;
  uint8_t* exp_buf = read_file(exp_path, &exp_len);
  if (!exp_buf && exp_len == 0) {
    fprintf(stderr, "failed to read expected: %s\n", exp_path);
    free(in_buf);
    return 2;
  }

  static uint8_t arena[64 * 1024 * 1024];

  bytes_t out = x07_solve_v2(
      arena,
      (uint32_t)sizeof(arena),
      (const uint8_t*)in_buf,
      (uint32_t)in_len
  );

  int ok = bytes_eq(out.ptr, (size_t)out.len, exp_buf, exp_len);

  if (!ok) {
    fprintf(stderr,
            "mismatch: input_len=%zu output_len=%u expected_len=%zu\n",
            in_len, out.len, exp_len);
    free(in_buf);
    free(exp_buf);
    return 1;
  }

  free(in_buf);
  free(exp_buf);
  return 0;
}

