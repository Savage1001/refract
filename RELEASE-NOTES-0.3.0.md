# Refract v0.3.0

This release includes the recently merged distribution, project trust, dependency, PDF scrolling, and workspace polish changes.

## Changes

- Refract no longer bundles TeX or SyncTeX. Install TeX Live or TinyTeX separately, set `REFRACT_TEX_BIN` to its Windows executable directory, and validate it with `scripts/setup-tex.ps1`. The installer and packaged app are checked to exclude TeX binaries and archives.
- Imported, duplicated, and legacy projects require explicit local trust before compilation or assistant execution. Review and cancellation are supported, and trust can be revoked. TeX and assistants still run with the user's account access; this is informed consent, not an OS filesystem sandbox.
- Selected UTF-8 text attachments include bounded request previews (16 KiB per file, 32 KiB total), avoiding provider shell startup for small documents. Truncation is explicit; larger and binary documents still depend on provider file tools.
- OpenCode 2 receives its temporary Refract MCP tools through private standalone configuration. Resuming clears the conversation's prior execution state so a stale provider claim cannot indefinitely queue the next turn. Global service settings are unchanged.
- PDF pages remain populated during fast scrolling. Workspace controls, branding placement, sidebar layout, and keyboard workflows have been polished.
- Build dependencies resolve the audited minimatch vulnerability without changing the Windows installer configuration.
- Windows CI runs automatically only when publishing a release, with a manual release-verification option. The test-only external TeX and SyncTeX downloads are checksum verified and never packaged or uploaded. Ordinary pushes and PRs do not trigger CI.

## Verification

- TypeScript build passed; full and production dependency audits report zero known vulnerabilities.
- The latest full hidden Windows run exercised 290 cases: 275 passed, 13 skipped, and two initial cache-hit assertions failed. Cache test setup now fixes the clock and allows bounded priming while Windows settles fresh file metadata; it still requires real reuse and verifies invalidation and trust revocation. The affected checks passed on rerun and in 30 successive repetitions. Post-fix service regressions passed 60 tests with two opt-in checks skipped. The final focused cache, trust, and OpenCode checks passed all 19 tests.
- All nine packaged checks passed: TeX exclusion, runtime dependency exclusions, real compilation, assistant formatting, source/PDF navigation, archive round trips, themes, missing-TeX startup, and trust approval/cancellation/revocation.
- All four live Codex service checks passed. All three live checks against the packaged app passed, including mixed attachments, session resumption after restart, and cancellation preserving a partial edit.
- Live Codex and OpenCode direct-MCP resumption passed, including fresh tool connections between turns. Real missing-package repair/build passed with both providers. The exact CI toolchain passed Unicode SyncTeX navigation across all three engines.
- Both final installer checks passed against the exact release bytes: silent error handling and the install/upgrade/uninstall lifecycle, including refusal to upgrade a running app and preservation of project data.

The Windows installer is unsigned and requires Windows. TeX and assistant CLIs are installed and authenticated separately. External provider shell commands can still stall in some Windows Codex environments; full document access outside the supplied text preview depends on the provider's functioning file tools. Physical network-disconnection testing and every possible external TeX installation are outside this verification.

Historical installers remain private drafts. They are not installers for this revision. The original private repository retains its historical binary-containing refs; public source publication must use the separately verified cleaned history/source copy.
