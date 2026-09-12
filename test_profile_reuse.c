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

int main(int argc, char **argv) {
  long N = atoi(argv[1]);
  int32_t *a_data = malloc(sizeof(int32_t) * (long)N * (long)N);
  HAD *a = malloc(sizeof(HAD));
  a->buf = a_data;
  a->dims[0] = N;
  a->dims[1] = N;
  a->strides[0] = N;
  a->strides[1] = 1;
  a->offset = 0;

  double t = 0;
  for (int iter = 0; iter < 5; iter++) {
    clock_t begin = clock();
    HAD *res = array_add_strict2(a, a);
    clock_t end = clock();
    t = (double)(end - begin) / CLOCKS_PER_SEC;
    free(res->buf);
    free(res);
  }

  printf("(%ld, %f),\n", N, t);
  return 0;
}
