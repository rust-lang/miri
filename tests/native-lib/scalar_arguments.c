#include <stdio.h>
#include <stdint.h>

// See comments in build_native_lib()
#define EXPORT __attribute__((visibility("default")))

EXPORT int32_t add_one_int(int32_t x) {
  return 2 + x;
}

EXPORT void printer(void) {
  printf("printing from C\n");
}

// function with many arguments, to test functionality when some args are stored
// on the stack
EXPORT int32_t test_stack_spill(int32_t a, int32_t b, int32_t c, int32_t d, int32_t e, int32_t f, int32_t g, int32_t h, int32_t i, int32_t j, int32_t k, int32_t l) {
  return a+b+c+d+e+f+g+h+i+j+k+l;
}

EXPORT uint32_t get_unsigned_int(void) {
  return -10;
}

EXPORT int16_t add_int16(int16_t x) {
  return x + 3;
}

EXPORT int64_t add_short_to_long(int16_t x, int64_t y) {
  return x + y;
}

/* Test: test_pass_struct */

typedef struct PassMe {
    int32_t value;
    int16_t other_value;
} PassMe;

EXPORT int32_t pass_struct(const PassMe pass_me) {
  return pass_me.value + pass_me.other_value;
}

/* Test: test_pass_struct_complex */

typedef struct Part1 {
    uint16_t high;
    uint16_t low;
} Part1;

typedef struct Part2 {
    uint32_t bits;
} Part2;

typedef struct ComplexStruct {
    Part1 part_1;
    Part2 part_2;
    uint32_t part_3;
} ComplexStruct;

EXPORT int32_t pass_struct_complex(const ComplexStruct complex) {
  if ((((uint32_t)complex.part_1.high) << 16 | (uint32_t)complex.part_1.low) == complex.part_2.bits
      && complex.part_2.bits == complex.part_3)
    return 0;
  else {
    return 1;
  }
}

// To test that functions not marked with EXPORT cannot be called by Miri.
int32_t not_exported(void) {
  return 0;
}
