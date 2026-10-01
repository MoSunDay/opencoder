#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/types.h>

int main(int argc, char **argv) {
    if (argc < 2) return 2;
    if (!strcmp(argv[1], "write")) {
        FILE *file = fopen("source.txt", "w");
        if (!file) return 3;
        if (fputs("changed\n", file) < 0 || fclose(file)) return 4;
        return 0;
    }
    if (!strcmp(argv[1], "read")) {
        char text[128];
        FILE *file = fopen("/workspace/writer/source.txt", "r");
        if (!file || !fgets(text, sizeof(text), file)) return 5;
        fclose(file);
        fputs(text, stdout);
        return strcmp(text, "changed\n") ? 6 : 0;
    }
    if (!strcmp(argv[1], "argv")) {
        if (argc != 5 || strcmp(argv[2], "with space") || strcmp(argv[3], "quoted \"value\"") || strcmp(argv[4], "")) return 7;
        puts("argument boundaries preserved");
        return 0;
    }
    if (!strcmp(argv[1], "spin")) {
        pid_t child = fork();
        if (child == 0) { setsid(); for (;;) pause(); }
        FILE *file = fopen("child.pid", "w");
        if (!file) return 8;
        fprintf(file, "%d", child);
        fclose(file);
        for (;;) pause();
    }
    if (!strcmp(argv[1], "overflow")) {
        char bytes[65536];
        memset(bytes, 'x', sizeof(bytes));
        for (int chunk = 0; chunk < 150; chunk++) fwrite(bytes, sizeof(bytes), 1, stdout);
        return 0;
    }
    return 9;
}
