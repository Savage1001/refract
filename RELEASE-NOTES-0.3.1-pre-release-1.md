# Refract v0.3.1-pre-release-1

Includes PRs #1–#3: README branding, release naming rules, automatic update checks and downloads, Stable/Beta tracks in home settings, and installation only through **Restart to update**. Supports `pre-release-N` versions on the Beta track.

Windows installer included. Build, updater tests, production audit, and silent-installer checks passed. The full run had 284 passes, 14 skips, and two failures; both affected test files passed separate rechecks (restore used workspace-local temporary storage). The install/upgrade/uninstall lifecycle check was skipped to protect an existing installation.
