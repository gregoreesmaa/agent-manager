/* Fake `muse` for the hermetic Windows live test (issue #64).
 *
 * The core's public spawn runs the real `muse` command; the live test
 * must not need one. `tests/smoke_live.ps1` compiles this file with the
 * runner's MSVC (`cl`) to `muse.exe`, puts it first on PATH, and runs
 * `staap-win-smoke --smoke-live` — the same hermetic trick as the core's
 * `public_spawn_success_path` test and the Linux `smoke_live.sh`, but
 * with a real PE: CreateProcess cannot execute the shell-script fake
 * the unix harnesses use.
 *
 * Behavior: print the readiness marker, then sleep 30s so the harness
 * has time to pump, write, and resize before the child exits.
 */

#include <stdio.h>

#ifdef _WIN32
#include <windows.h>
int main(void) {
    puts("fake-muse-ready-64");
    fflush(stdout);
    Sleep(30000);
    return 0;
}
#else
#include <unistd.h>
int main(void) {
    puts("fake-muse-ready-64");
    fflush(stdout);
    sleep(30);
    return 0;
}
#endif
