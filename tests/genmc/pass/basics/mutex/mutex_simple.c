#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <assert.h>

#define REPS 1

static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static uint64_t data[32];

void* thread_1(void* arg) {
    for (uint64_t i = 0; i < REPS; i++) {
        pthread_mutex_lock(&lock);
        data[0] += 2;
        pthread_mutex_unlock(&lock);
    }
    return NULL;
}

void* thread_2(void* arg) {
    for (uint64_t i = 0; i < REPS; i++) {
        pthread_mutex_lock(&lock);
        data[0] += 4;
        pthread_mutex_unlock(&lock);
    }
    return NULL;
}

int main(int argc, char** argv) {
    // Initialize data
    for (int i = 0; i < 32; i++) {
        data[i] = 1234;
    }

    pthread_mutex_lock(&lock);
    for (int i = 0; i < 32; i++) {
        assert(data[i] == 1234);
    }
    data[0] = 0;
    data[1] = 10;
    assert(data[0] == 0 && data[1] == 10);
    pthread_mutex_unlock(&lock);

    // Thread order: can be changed for different test orders
#ifdef ORDER21
    void* (*thread_order[2])(void*) = {thread_2, thread_1};
#else
    void* (*thread_order[2])(void*) = {thread_1, thread_2};
#endif

    pthread_t ids[2];
    for (int i = 0; i < 2; i++) {
        int ret = pthread_create(&ids[i], NULL, thread_order[i], NULL);
        assert(ret == 0);
    }
    
    for (int i = 0; i < 2; i++) {
        int ret = pthread_join(ids[i], NULL);
        assert(ret == 0);
    }

    pthread_mutex_lock(&lock);
    // assert(data[0] == REPS * 6); // Not checked, but can be enabled
    assert(data[1] == 10);
    for (int i = 2; i < 32; i++) {
        assert(data[i] == 1234);
    }
    pthread_mutex_unlock(&lock);

    return 0;
}
