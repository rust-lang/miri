#include <stdio.h>
#include <stdlib.h>

void ddref_print(int **x) {
  printf("PTR %d\n", x);
  printf("*PTR %d\n", *x);
  printf("**PTR %d\n", **x);
}

int get_num(int x) {
  return 2 + x;
}

int* pointer_test() {
      int *point = malloc(sizeof(int)); 
      *point=1;  
      return point;
}

void ptr_printer(int *x) {
  printf("pointer has value: %d\n", *x);
}

double get_dbl(int x) {
  return 2.75 + x;
}

void printer() {
  printf("printing from C\n");
}

int test_stack_spill(int a, int b, int c, int d, int e, int f, int g, int h, int i, int j, int k, int l) {
  return a+b+c+d+e+f+g+h+i+j+k+l;
}
