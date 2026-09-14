#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

#define RANK 2

typedef struct {
  int32_t *buf;
  uint64_t offset;
  uint64_t strides[RANK];
  uint64_t dims[RANK];
} HAD;

extern HAD *array_add_strict2(HAD *a, HAD *b);

static HAD *make(uint64_t rows, uint64_t cols, int32_t *data) {
  HAD *d = malloc(sizeof(HAD));
  d->buf = malloc(rows * cols * sizeof(int32_t));
  for (uint64_t i = 0; i < rows * cols; i++)
    d->buf[i] = data[i];
  d->offset = 0;
  d->strides[0] = cols;
  d->strides[1] = 1;
  d->dims[0] = rows;
  d->dims[1] = cols;
  return d;
}

int main(int argc, char **argv) {
  long N = atoi(argv[1]);
  int32_t *a_data = malloc(sizeof(int32_t) * (long)N * (long)N);
  for (long i = 0; i < (long)N * (long)N; i++) {
    a_data[i] = i;
  }

  HAD *a = make(N, N, a_data);

  clock_t begin = clock();
  // for (int i = 0; i < 100; i++) {
    HAD *result = array_add_strict2(a, a);
  // }
  clock_t end = clock();
  double time_spent = (double)(end - begin) / CLOCKS_PER_SEC;

  printf("%ld\t%f\n", N, time_spent);

  return 0;
}
