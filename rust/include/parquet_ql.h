#ifndef PARQUET_QL_H
#define PARQUET_QL_H
#include <stdint.h>

/// Renders a Parquet file as a self-contained HTML page (UTF-8, NUL-terminated).
/// Audio cells contain <audio> players with data: URIs. Errors are returned as an HTML error page,
/// never NULL (except on OOM). max_rows == 0 → default (500).
/// Free the result with pq_free_string.
char *pq_render_html(const char *path, uint32_t max_rows);
void pq_free_string(char *s);

#endif
