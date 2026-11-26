#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>

extern int32_t oreslang_scalar_entry(int64_t, int64_t, int64_t, int64_t, int64_t*);

/* Independent widened oracle: no signed-i64 overflow in the test itself. */
static int check(int64_t a, int64_t b, int32_t want, int64_t value) {
    const int64_t sentinel = INT64_C(0x1122334455667788);
    int64_t out = sentinel;
    int32_t status = oreslang_scalar_entry(a, b, 0, 0, &out);
    if (status != want || out != (want == 0 ? value : sentinel)) {
        fprintf(stderr, "unexpected native outcome: status=%d expected=%d\n",
                (int)status, (int)want);
        return 1;
    }
    return 0;
}
int main(int argc, char **argv) {
    if (argc != 2) return 2;
    int mode = atoi(argv[1]);
    if (mode == 0) {
        const int64_t nums[] = {INT64_MIN, INT64_MIN+1, -INT64_C(9007199254740993),
            -7, -1, 0, 1, 7, INT64_C(9007199254740993), INT64_MAX-1, INT64_MAX};
        for (size_t i=0; i<sizeof(nums)/sizeof(nums[0]); ++i)
            for (size_t j=0; j<sizeof(nums)/sizeof(nums[0]); ++j) {
                int64_t a=nums[i], b=nums[j];
                if (b==0) { if (check(a,b,2,0)) return 1; }
                else {
                    __int128 q=(__int128)a / (__int128)b;
                    if (q<INT64_MIN || q>INT64_MAX) {
                        if (check(a,b,1,0)) return 1;
                    } else if (check(a,b,0,(int64_t)q)) return 1;
                }
            }
    } else if (mode == 1) {
        /* (a+1)/b: left operand failure takes precedence over /0. */
        if (check(INT64_MAX,0,1,0) || check(41,0,2,0) ||
            check(83,2,0,42) || check(-8,2,0,-3)) return 1;
    } else if (mode == 2) {
        /* seq(a/b,MAX+1): division failure precedes later overflow. */
        if (check(7,0,2,0) || check(INT64_MIN,-1,1,0) ||
            check(7,2,1,0)) return 1;
    } else if (mode == 3) {
        /* Discarded runtime local must still evaluate its initializer. */
        if (check(7,0,2,0) || check(INT64_MIN,-1,1,0) ||
            check(7,2,0,42) || check(-7,2,0,42)) return 1;
    } else if (mode == 4) {
        /* a/(b+1): the divisor expression can fault before division. */
        if (check(7,INT64_MAX,1,0) || check(7,-1,2,0) ||
            check(INT64_MIN,-2,1,0) || check(INT64_MIN,0,0,INT64_MIN) ||
            check(85,1,0,42) || check(-85,1,0,-42)) return 1;
    } else if (mode == 5) {
        /* seq(a+1, a/b): earlier overflow must win over later /0. */
        if (check(INT64_MAX,0,1,0) || check(7,0,2,0) ||
            check(INT64_MIN,-1,1,0) || check(85,2,0,42) ||
            check(-85,2,0,-42)) return 1;
    } else if (mode == 6) {
        /* (a+1)/(b+1): both operands are checked before quotient. */
        if (check(INT64_MAX,-1,1,0) || check(7,INT64_MAX,1,0) ||
            check(7,-1,2,0) || check(INT64_MIN,-2,0,INT64_MAX) ||
            check(83,1,0,42) || check(-86,1,0,-42)) return 1;
    } else return 2;
    puts("CHECKED_DIVISION_AOT_ORDER_OK");
    return 0;
}
