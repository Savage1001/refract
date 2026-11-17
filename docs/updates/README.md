# Application updates

The installed Windows app checks Savage1001/refract releases at startup and every ten minutes. It downloads newer versions automatically, verifies the installer through electron-updater (including the metadata SHA-512 checksum), and installs silently on normal app exit after the existing save/compiler/assistant shutdown barrier. Development runs never download or install updates.

The gear beside Import on the project home page opens Settings. Use **Check for updates** to run a check immediately on the selected track; repeated clicks are disabled while it checks or downloads. Stable is the default and excludes prereleases. Beta selects the highest semantic version among stable releases and GitHub prereleases tagged `vX.Y.Z-beta.N`. Drafts, alpha/RC releases, and older/equal versions are ignored. Switching from a beta to stable waits for a newer stable release; it never downgrades. The preference is stored in the app data directory and survives restart. Failed checks or downloads retry on the next interval.

## Publishing compatible releases

Build with `npm run build` then `node node_modules/electron-builder/cli.js --win nsis --publish never`. Publish the generated NSIS `.exe`, its `.blockmap`, and all generated channel `.yml` files together on the matching GitHub release. Uploaded asset names must match the URLs in the channel metadata exactly (for example `Refract-Setup-X.Y.Z.exe`). The package version must match its release tag. Stable versions need `latest.yml`; beta versions need `beta.yml` (architecture suffixes apply to ARM64). `generateUpdatesFilesForAllChannels` enables the channel metadata. Do not publish a release until these assets are available. Existing releases without metadata report a retryable error. No release or installer is published by this PR.

Release downloads use the public repository without a bundled credential. A private repository requires a separately designed authenticated distribution service; never embed a GitHub token in the app.

## Evidence

`home-settings-icon.png` shows the home-only gear; `settings-menu.png` shows Beta selected. Both use the real renderer with isolated sample data in headless Chromium. Tests cover semantic release ordering, saved preferences, retries, failed downloads, metadata mismatch, switching tracks while a download is in flight, and home-only settings visibility. A full live installed-app upgrade still requires a newer published release with updater metadata.
