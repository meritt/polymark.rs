#ifndef POLYMARK_H
#define POLYMARK_H

#include <stddef.h>
#include <stdint.h>

#define POLYMARK_OK 0

#define POLYMARK_INVALID_UTF8 1

#define POLYMARK_INVALID_ARGUMENT 2

#define POLYMARK_INTERNAL 3

#define POLYMARK_PLAIN 0

#define POLYMARK_MARKDOWN 1

#define POLYMARK_HTML 2

#define POLYMARK_RICH 3

#define POLYMARK_TELEGRAM 4

typedef struct polymark_report {
  uint32_t status;
  uint8_t *data;
  size_t len;
} polymark_report;

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

void polymark_transform(const uint8_t *input,
                        size_t input_len,
                        uint32_t from,
                        uint32_t to,
                        struct polymark_report *out);

void polymark_clean(const uint8_t *input, size_t input_len, struct polymark_report *out);

void polymark_report_free(struct polymark_report *report);

const char *polymark_version(void);

uint8_t *polymark_alloc(size_t len);

void polymark_dealloc(uint8_t *p, size_t len);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* POLYMARK_H */
