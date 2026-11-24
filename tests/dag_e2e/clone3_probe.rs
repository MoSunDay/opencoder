//! The in-sandbox environment probe: a static binary step printing one JSON
//! line of kernel-enforced facts about its sandbox — credentials and
//! capabilities, PID/namespace placement, read-only and writable paths,
//! masked files, devices and the mount table it actually sees.

pub const PROBE_C: &str = r##"
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/statvfs.h>
#include <unistd.h>

static void slurp(const char *path, char *out, size_t cap) {
    out[0] = 0;
    int fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) return;
    ssize_t n = read(fd, out, cap - 1);
    close(fd);
    out[n > 0 ? (size_t)n : 0] = 0;
}

/* Value of a /proc/<pid>/status field; tabs and runs of spaces collapse. */
static void field(const char *status, const char *name, char *out, size_t cap) {
    out[0] = 0;
    size_t len = strlen(name);
    for (const char *line = status; line && *line; ) {
        const char *end = strchr(line, '\n');
        size_t size = end ? (size_t)(end - line) : strlen(line);
        if (size > len && strncmp(line, name, len) == 0 && line[len] == ':') {
            const char *value = line + len + 1;
            size_t left = size - len - 1, used = 0;
            while (left && (*value == ' ' || *value == '\t')) { value++; left--; }
            for (size_t i = 0; i < left && used + 1 < cap; i++) {
                char c = value[i] == '\t' ? ' ' : value[i];
                if (used && c == ' ' && out[used - 1] == ' ') continue;
                out[used++] = c;
            }
            while (used && out[used - 1] == ' ') used--;
            out[used] = 0;
            return;
        }
        line = end ? end + 1 : NULL;
    }
}

static long long ns_inode(const char *path) {
    char target[256];
    ssize_t n = readlink(path, target, sizeof(target) - 1);
    if (n <= 0) return -1;
    target[n] = 0;
    const char *open = strchr(target, '[');
    return open ? strtoll(open + 1, NULL, 10) : -1;
}

/* errno of a failed exclusive create; 0 when it worked (the file is removed again). */
static int probe_write(const char *path) {
    int fd = open(path, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0600);
    if (fd < 0) return errno;
    close(fd);
    unlink(path);
    return 0;
}

static int rdonly(const char *path) {
    struct statvfs fs;
    return statvfs(path, &fs) == 0 ? ((fs.f_flag & ST_RDONLY) ? 1 : 0) : -1;
}

static void status_json(const char *status) {
    char uid[64], gid[64], bnd[64], eff[64], prm[64], amb[64], nnp[16], nspid[64];
    field(status, "Uid", uid, sizeof(uid));
    field(status, "Gid", gid, sizeof(gid));
    field(status, "CapBnd", bnd, sizeof(bnd));
    field(status, "CapEff", eff, sizeof(eff));
    field(status, "CapPrm", prm, sizeof(prm));
    field(status, "CapAmb", amb, sizeof(amb));
    field(status, "NoNewPrivs", nnp, sizeof(nnp));
    field(status, "NSpid", nspid, sizeof(nspid));
    printf("{\"uid\":\"%s\",\"gid\":\"%s\",\"cap_bnd\":\"%s\",\"cap_eff\":\"%s\","
           "\"cap_prm\":\"%s\",\"cap_amb\":\"%s\",\"no_new_privs\":\"%s\",\"nspid\":\"%s\"}",
           uid, gid, bnd, eff, prm, amb, nnp, nspid);
}

static void ns_json(const char *who) {
    static const char *kinds[] = {"mnt", "pid", "uts", "ipc", "net", "user", "cgroup"};
    char path[128];
    printf("{");
    for (size_t i = 0; i < 7; i++) {
        snprintf(path, sizeof(path), "/proc/%s/ns/%s", who, kinds[i]);
        printf("%s\"%s\":%lld", i ? "," : "", kinds[i], ns_inode(path));
    }
    printf("}");
}

/* The mount table the step sees: point, filesystem type and ro flag. */
static void mounts_json(void) {
    FILE *stream = fopen("/proc/self/mountinfo", "r");
    char line[4096];
    int first = 1;
    printf("[");
    while (stream && fgets(line, sizeof(line), stream)) {
        char *separator = strstr(line, " - ");
        if (!separator) continue;
        *separator = 0;
        char *save = NULL, *point = NULL, *options = NULL;
        int index = 0;
        for (char *token = strtok_r(line, " ", &save); token;
             token = strtok_r(NULL, " ", &save), index++) {
            if (index == 4) point = token;
            if (index == 5) options = token;
        }
        char *kind = separator + 3;
        kind[strcspn(kind, " \n")] = 0;
        if (!point || !options) continue;
        int ro = 0;
        char *option_save = NULL;
        for (char *token = strtok_r(options, ",", &option_save); token;
             token = strtok_r(NULL, ",", &option_save))
            if (strcmp(token, "ro") == 0) ro = 1;
        printf("%s{\"point\":\"%s\",\"kind\":\"%s\",\"ro\":%s}", first ? "" : ",", point, kind,
               ro ? "true" : "false");
        first = 0;
    }
    if (stream) fclose(stream);
    printf("]");
}

int main(void) {
    char status[8192], init[8192], comm[64], file[256], link[256], exe[1024];
    slurp("/proc/self/status", status, sizeof(status));
    slurp("/proc/1/status", init, sizeof(init));
    slurp("/proc/1/comm", comm, sizeof(comm));
    comm[strcspn(comm, "\n")] = 0;
    slurp("/etc/hostname", file, sizeof(file));
    file[strcspn(file, "\n")] = 0;
    char host[256] = {0};
    gethostname(host, sizeof(host) - 1);
    ssize_t n = readlink("/dev/ptmx", link, sizeof(link) - 1);
    link[n > 0 ? (size_t)n : 0] = 0;
    n = readlink("/proc/self/exe", exe, sizeof(exe) - 1);
    exe[n > 0 ? (size_t)n : 0] = 0;
    struct stat st;
    long long kcore = stat("/proc/kcore", &st) == 0 ? (long long)st.st_size : -1;
    int fd = open("/dev/null", O_RDWR | O_CLOEXEC);
    int dev_null = fd >= 0;
    if (dev_null) close(fd);
    fd = open("/dev/ptmx", O_RDWR | O_NOCTTY | O_CLOEXEC);
    int dev_ptmx = fd >= 0;
    if (dev_ptmx) close(fd);
    int knowledge = access("/run/opencoder/knowledge", F_OK) == 0;
    printf("{\"pid\":%d,\"uid\":%u,\"euid\":%u,\"gid\":%u,\"egid\":%u,\"status\":",
           getpid(), getuid(), geteuid(), getgid(), getegid());
    status_json(status);
    printf(",\"init_comm\":\"%s\",\"init_status\":", comm);
    status_json(init);
    printf(",\"hostname\":\"%s\",\"etc_hostname\":\"%s\",\"exe\":\"%s\",\"ns\":", host, file, exe);
    ns_json("self");
    printf(",\"init_ns\":");
    ns_json("1");
    printf(",\"kcore_size\":%lld,\"dev_null\":%s,\"dev_ptmx\":%s,\"ptmx_link\":\"%s\",", kcore,
           dev_null ? "true" : "false", dev_ptmx ? "true" : "false", link);
    printf("\"rdonly\":{\"root\":%d,\"proc_sys\":%d,\"etc\":%d,\"tmp\":%d,\"workspace\":%d,\"knowledge\":%d},",
           rdonly("/"), rdonly("/proc/sys"), rdonly("/etc"), rdonly("/tmp"), rdonly("/workspace"),
           knowledge ? rdonly("/run/opencoder/knowledge") : -1);
    printf("\"write\":{\"root\":%d,\"proc_sys\":%d,\"etc\":%d,\"tmp\":%d,\"workspace\":%d,\"knowledge\":%d},",
           probe_write("/.write-probe"), probe_write("/proc/sys/kernel/.write-probe"),
           probe_write("/etc/.write-probe"), probe_write("/tmp/.write-probe"),
           probe_write("/workspace/.write-probe"),
           knowledge ? probe_write("/run/opencoder/knowledge/.write-probe") : -1);
    printf("\"mounts\":");
    mounts_json();
    printf("}\n");
    return ferror(stdout) ? 1 : 0;
}
"##;
