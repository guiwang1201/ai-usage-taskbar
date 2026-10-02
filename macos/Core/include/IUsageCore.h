#ifndef IUSAGE_CORE_H
#define IUSAGE_CORE_H
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include <sys/file.h>
static inline int iusage_flock(int fd, int operation) { return flock(fd, operation); }
typedef struct { double five_hour, seven_day; int64_t five_reset, seven_reset; uint32_t present; } MacUsage;
typedef struct { int64_t last_attempt, next_poll, blocked_until, backoff; } MacGate;
int32_t iusage_parse(uint32_t provider, const uint8_t *bytes, size_t len, MacUsage *out);
bool iusage_can_request(const MacGate *gate, int64_t now);
bool iusage_start(MacGate *gate, int64_t now);
bool iusage_complete(MacGate *gate, int64_t now, uint16_t status, uint32_t retry_after, uint32_t poll_secs);
#endif
