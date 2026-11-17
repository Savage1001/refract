# Application updates

The installed Windows app checks Savage1001/refract releases at startup and every ten minutes. It downloads newer versions automatically, verifies the installer through electron-updater (including the metadata SHA-512 checksum), and waits for the user to click **Restart to update** in Settings before installing and relaunching. The existing save/compiler/assistant shutdown barrier finishes before installation starts. Closing the app normally never installs an update. Development runs never download or install updates.

The gear beside Import on the project home page opens Settings. Use **Check for updates** to run a check immediately on the selected track; repeated clicks are disabled while it checks or downloads. Stable is the default and excludes prereleases. Beta selects the highest semantic version among stable releases and GitHub prereleases tagged `vX.Y.Z-beta.N`. Drafts, alpha/RC releases, and older/equal versions are ignored. Switching from a beta to stable waits for a newer stable release; it never downgrades. The preference is stored in the app data directory and survives restart. Failed checks or downloads retry on the next interval.

## Publishing compatible releases

Build with `npm run build` then `node node_modules/electron-builder/cli.js --win nsis --publish never`. Publish the generated NSIS `.exe`, its `.blockmap`, and all generated channel `.yml` files together on the matching GitHub release. Uploaded asset names must match the URLs in the channel metadata exactly (for example `Refract-Setup-X.Y.Z.exe`). The package version must match its release tag. Stable versions need `latest.yml`; beta versions need `beta.yml` (Windows channel filenames have no architecture suffix, including ARM64). `generateUpdatesFilesForAllChannels` enables the channel metadata. Do not publish a release until these assets are available. Existing releases without metadata report a retryable error. No release or installer is published by this PR.

Release downloads use the public repository without a bundled credential. A private repository requires a separately designed authenticated distribution service; never embed a GitHub token in the app.

## Evidence

`home-settings-icon.png` shows the home-only gear; `settings-menu.png` shows Beta selected. `update-available.png` shows the Beta track downloading an update at 64%; `update-ready.png` shows the downloaded update and the **Restart to update** button. These use the real renderer with simulated update status and isolated sample data in headless Chromium. The restart button appears only when a downloaded update is ready; Settings Close and normal app exits never apply updates. Tests cover semantic release ordering, saved preferences, retries, failed downloads, metadata mismatch, switching tracks while a download is in flight, and home-only settings visibility. A full live installed-app upgrade still requires a newer published release with updater metadata.

## Reliability audit

Regression tests reject malformed beta tags and prevent restart while a track change is pending, including before preference I/O finishes. A hidden Electron integration test uses the real NSIS updater and a local HTTP feed to reject a corrupt installer by SHA-512, download a valid inert fixture, reuse its verified cache, and inspect silent-install/relaunch arguments without executing an installer. It also checks that ARM64 accepts the unsuffixed Windows metadata filename and that downloads register no automatic quit installation. These checks do not replace a live upgrade between published installed versions.
