// Thin Xbox Live shim — replace with GDK/XGameRuntime calls when Partner
// Center sandbox credentials are wired. Compiles on all targets so CI can
// build the windows-store feature on macOS/Linux.

#include "xbox_shim.h"

#include <cstdio>

static bool g_initialized = false;

bool xbox_init(void) {
    if (g_initialized) {
        return true;
    }
#if defined(_WIN32)
    // TODO: XGameRuntimeInit + XUserAddAsync when GDK is linked in MSIX package.
    std::fprintf(stderr, "xbox_shim: stub init (wire GDK for production MSIX)\n");
#endif
    g_initialized = true;
    return true;
}

bool xbox_unlock_achievement(const char *id) {
    if (!g_initialized || id == nullptr) {
        return false;
    }
#if defined(_WIN32)
    std::fprintf(stderr, "xbox_shim: unlock '%s' (stub)\n", id);
#endif
    (void)id;
    return true;
}

bool xbox_set_stat_i32(const char *name, int value) {
    if (!g_initialized || name == nullptr) {
        return false;
    }
#if defined(_WIN32)
    std::fprintf(stderr, "xbox_shim: stat '%s' = %d (stub)\n", name, value);
#endif
    (void)name;
    (void)value;
    return true;
}

void xbox_flush_stats(void) {}
