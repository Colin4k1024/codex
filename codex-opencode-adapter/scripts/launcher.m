// Thin wrapper launcher for Codex .app bundle.
// macOS requires CFBundleExecutable to be a Mach-O binary (not a shell script).
// This program execve's the codex-opencode-adapter in the same directory.
#include <unistd.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <libgen.h>
#include <mach-o/dyld.h>
#include <spawn.h>
#include <sys/wait.h>

extern char **environ;

int main(int argc, char *argv[]) {
    char exe_path[4096];
    uint32_t size = sizeof(exe_path);
    if (_NSGetExecutablePath(exe_path, &size) != 0) {
        fprintf(stderr, "failed to get executable path\n");
        return 1;
    }

    char *dir = dirname(exe_path);
    char adapter_path[4096];
    snprintf(adapter_path, sizeof(adapter_path), "%s/codex-opencode-adapter", dir);

    // Set environment for adapter
    setenv("PORT", "0", 0);
    setenv("HOSTNAME", "127.0.0.1", 0);

    char *new_argv[] = { adapter_path, NULL };
    pid_t pid;
    int ret = posix_spawn(&pid, adapter_path, NULL, NULL, new_argv, environ);
    if (ret != 0) {
        fprintf(stderr, "failed to spawn adapter: %s\n", strerror(ret));
        return 1;
    }

    // Wait for adapter process
    int status;
    waitpid(pid, &status, 0);
    return WIFEXITED(status) ? WEXITSTATUS(status) : 1;
}