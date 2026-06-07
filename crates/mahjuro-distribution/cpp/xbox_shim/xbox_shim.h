#pragma once

#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

bool xbox_init(void);
bool xbox_unlock_achievement(const char *id);
bool xbox_set_stat_i32(const char *name, int value);
void xbox_flush_stats(void);

#ifdef __cplusplus
}
#endif
