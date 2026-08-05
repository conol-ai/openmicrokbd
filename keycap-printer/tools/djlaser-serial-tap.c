#define _DARWIN_C_SOURCE

#include <fcntl.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/time.h>
#include <unistd.h>

extern ssize_t __read_nocancel(int fd, void *buffer, size_t length);
extern ssize_t __write_nocancel(int fd, const void *buffer, size_t length);

static int capture_fd = -1;
static pthread_mutex_t capture_lock = PTHREAD_MUTEX_INITIALIZER;

#define DYLD_INTERPOSE(replacement, replacee) \
    __attribute__((used)) static struct { \
        const void *replacement; \
        const void *replacee; \
    } interpose_##replacee __attribute__((section("__DATA,__interpose"))) = { \
        (const void *)(uintptr_t)&replacement, \
        (const void *)(uintptr_t)&replacee \
    }

static void raw_write_all(const char *data, size_t length)
{
    while (length > 0) {
        ssize_t written = __write_nocancel(capture_fd, data, length);
        if (written <= 0)
            return;
        data += written;
        length -= (size_t)written;
    }
}

static bool serial_path_for_fd(int fd, char *path, size_t path_size)
{
    if (fd < 0 || path_size == 0 || fcntl(fd, F_GETPATH, path) != 0)
        return false;
    path[path_size - 1] = '\0';
    return strstr(path, "/dev/cu.usbmodem") != NULL || strstr(path, "/dev/tty.usbmodem") != NULL;
}

static void capture_bytes(const char *direction, int fd, const void *buffer, size_t length)
{
    if (capture_fd < 0 || buffer == NULL || length == 0)
        return;

    char path[1024] = {0};
    if (!serial_path_for_fd(fd, path, sizeof(path)))
        return;

    const uint8_t *bytes = (const uint8_t *)buffer;
    struct timeval now;
    gettimeofday(&now, NULL);

    pthread_mutex_lock(&capture_lock);

    char line[2048];
    int line_length = snprintf(
        line,
        sizeof(line),
        "\n[%lld.%06d] %s fd=%d bytes=%zu path=%s\n",
        (long long)now.tv_sec,
        now.tv_usec,
        direction,
        fd,
        length,
        path
    );
    if (line_length > 0)
        raw_write_all(line, (size_t)line_length);

    for (size_t offset = 0; offset < length; offset += 16) {
        size_t count = length - offset;
        if (count > 16)
            count = 16;

        int used = snprintf(line, sizeof(line), "%08zx  ", offset);
        for (size_t index = 0; index < 16; index += 1) {
            used += snprintf(
                line + used,
                sizeof(line) - (size_t)used,
                index < count ? "%02x " : "   ",
                index < count ? bytes[offset + index] : 0
            );
        }
        used += snprintf(line + used, sizeof(line) - (size_t)used, " |");
        for (size_t index = 0; index < count; index += 1) {
            uint8_t value = bytes[offset + index];
            line[used++] = value >= 32 && value <= 126 ? (char)value : '.';
        }
        line[used++] = '|';
        line[used++] = '\n';
        raw_write_all(line, (size_t)used);
    }

    pthread_mutex_unlock(&capture_lock);
}

__attribute__((constructor))
static void initialize_tap(void)
{
    const char *capture_path = getenv("HY_LASER_TAP_LOG");
    if (capture_path == NULL)
        capture_path = "/tmp/djlaser-serial-capture.log";
    capture_fd = open(capture_path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (capture_fd >= 0)
        raw_write_all("DJLaser serial tap initialized\n", 31);
}

static ssize_t tap_read(int fd, void *buffer, size_t length)
{
    ssize_t result = __read_nocancel(fd, buffer, length);
    if (result > 0)
        capture_bytes("RX", fd, buffer, (size_t)result);
    return result;
}

static ssize_t tap_write(int fd, const void *buffer, size_t length)
{
    ssize_t result = __write_nocancel(fd, buffer, length);
    if (result > 0)
        capture_bytes("TX", fd, buffer, (size_t)result);
    return result;
}

DYLD_INTERPOSE(tap_read, read);
DYLD_INTERPOSE(tap_write, write);
