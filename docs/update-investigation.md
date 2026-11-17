# Windows update investigation

The November 17 report establishes that pre-release-3 was downloaded while
pre-release-2 remained installed, and that multiple independent main processes
shared the installation and user-data directory. It does not establish that the
update button successfully invoked the installer. A plain process restart does
not install an update because installation on ordinary quit is disabled.

The installed application now uses one main process per user-data directory;
another launch restores its existing window. Development and isolated test
instances retain their existing behavior. This application currently creates
one workspace window; the lock does not prohibit additional windows managed by
that main process. Existing orphan processes from older builds are not killed.

Shutdown keeps the window visible. Stage diagnostics include timestamps, PID,
installed version and elapsed durations. After 30 seconds a warning identifies
the outstanding stage; it does not terminate work or treat the operation as
finished. Failed shutdown returns to the app, restores renderer interaction,
and disarms the pending restart. Saves that fail prevent installation.

Application diagnostics: the user-data directory's `logs/updates.jsonl`.
Installer dependency failures: `%LOCALAPPDATA%/Refract/dependencies/setup-error.log`.
Updater logging records installer invocation and reported startup failures.
An attempt marker records the target version; the next startup logs whether
the running version matches it. Settings reports installed and ready versions.

Verified: TypeScript checking, stalled-stage diagnostics without cancellation
of work, installer handoff error/retry, download corruption and cache reuse,
release selection, track changes, and compiled NSIS environment cleanup.
The desktop queued-save shutdown check passed. The stalled-compiler desktop
check failed before shutdown because its LaTeX fixture did not reach the
deliberate loop; compiler shutdown remains unverified in this run.

Remaining acceptance work requires an isolated Windows installed-build run:
actual pre-release-2 to pre-release-3 replacement and usable relaunch, old process
tree exit, second-launch restoration, active assistant shutdown, and a real
binary lock. The updater API does not provide installer exit status after the
app exits; a persistent installer supervisor/exit log is still needed to fully
diagnose failures beyond the handoff. No exact original root cause is claimed.
