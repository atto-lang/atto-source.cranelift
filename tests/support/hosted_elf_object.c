/* Test-only ELF relocatable identity preflight. Not a loader, relocation,
 * ARM attribute, libc, or compiler execution proof. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static uint64_t le(const unsigned char *p, unsigned n) {
    uint64_t result = 0;
    for (unsigned i = 0; i < n; ++i) result |= (uint64_t)p[i] << (8 * i);
    return result;
}

static int validate(FILE *file, unsigned width, unsigned machine) {
    unsigned char h[64];
    const unsigned size = width == 64 ? 64 : 52;
    const unsigned word = width / 8;
    const unsigned section_size = width == 64 ? 64 : 40;
    if (fseek(file, 0, SEEK_END) != 0) return 0;
    const long length = ftell(file);
    if (length < (long)size || fseek(file, 0, SEEK_SET) != 0 ||
        fread(h, 1, size, file) != size) return 0;
    if (memcmp(h, "\177ELF", 4) || h[4] != (width == 64 ? 2 : 1) ||
        h[5] != 1 || h[6] != 1 || le(h + 16, 2) != 1 ||
        le(h + 18, 2) != machine || le(h + 20, 4) != 1 ||
        le(h + (width == 64 ? 52 : 40), 2) != size) return 0;
    if (le(h + 24, word) || le(h + (width == 64 ? 32 : 28), word) ||
        le(h + (width == 64 ? 56 : 44), 2)) return 0;
    if (machine == 40) {
        const uint64_t flags = le(h + 36, 4);
        const uint64_t fp = flags & 0x600;
        if ((flags >> 24) != 5 || (fp != 0 && fp != 0x400)) return 0;
    }
    const uint64_t start = le(h + (width == 64 ? 40 : 32), word);
    const uint64_t count = le(h + (width == 64 ? 60 : 48), 2);
    const uint64_t names = le(h + (width == 64 ? 62 : 50), 2);
    /* Bounded gate: no extended section numbering. Subtraction avoids overflow. */
    return start >= size && start <= (uint64_t)length && count != 0 &&
           names != 0 && names < count &&
           le(h + (width == 64 ? 58 : 46), 2) == section_size &&
           count * section_size <= (uint64_t)length - start;
}

int main(int argc, char **argv) {
    if (argc != 3) return 2;
    unsigned width, machine;
    if (!strcmp(argv[1], "x86_64")) { width = 64; machine = 62; }
    else if (!strcmp(argv[1], "aarch64")) { width = 64; machine = 183; }
    else if (!strcmp(argv[1], "x86")) { width = 32; machine = 3; }
    else if (!strcmp(argv[1], "arm")) { width = 32; machine = 40; }
    else { fprintf(stderr, "E_TARGET_OBJECT: unsupported test architecture\n"); return 2; }
    FILE *file = fopen(argv[2], "rb");
    if (!file) return 2;
    const int ok = validate(file, width, machine);
    fclose(file);
    if (!ok) fprintf(stderr, "E_TARGET_OBJECT: invalid or mismatched ELF relocatable\n");
    return ok ? 0 : 1;
}
