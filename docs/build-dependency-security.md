# Build dependency security

GHSA-g3hv-75wg-m249 affects minimatch >=7.0.0 <11.0.1. The Windows
packager uses minimatch through app-builder-lib; @electron/universal also
pulls an affected copy even though Refract only distributes Windows builds.
These are development dependencies. Keep full dependency audits as well as
production audits: production-only scanning misses the vulnerable build tools.

The published electron-builder 26.18.0 (and the v26-tagged 26.18.1) still
require affected minimatch versions. npm's proposed downgrade to 25.1.0
is not an appropriate security upgrade. The targeted npm overrides move the
two affected dependency ranges (^10.2.5 and ^9.0.3) to ^11.0.3. They leave
minimatch 3 and 5 used by older glob consumers alone.

Upstream's [v26 compatibility investigation](https://github.com/electron-userland/electron-builder/pull/10538)
confirms the FileMatcher APIs are compatible with minimatch 11 and warns
against blanket overrides or forcing brace-expansion below 5. The change
is still an open upstream proposal, so these are locally verified resolutions,
not a released upstream fix. minimatch 11 provides CommonJS exports;
its matching brace-expansion dependency must remain intact. @electron/universal
2.0.3 uses the named minimatch function with matchBase, also preserved in 11.
The build-dependencies test exercises both consumers' resolution, brace
expansion, and the actual application/resource filters. Remove the overrides
once supported upstream releases naturally resolve patched dependencies, and
rerun these compatibility checks. Use a supported Node 22 or 24 build runtime
(verification used Node 24); brace-expansion 5 requires Node 20 or newer.

electron-builder's main file matcher excludes node_modules from the ordinary
application-file walk; its separate node-module collector copies production
dependencies. The broad node_modules file pattern therefore does not itself
ship development tools. A windowless check fed `npm list` production output
into the actual NpmNodeModulesCollector: all eight direct runtime dependencies
were present among 100 collected modules, while electron-builder,
app-builder-lib, @electron/universal, and minimatch were absent. This checks the
collector, not a finished distribution artifact.

For a windowless verification, install with `npm ci --ignore-scripts`, run
TypeScript compilation, then run `node --test build/test/build-dependencies.test.cjs`.
Run both `npm audit --package-lock-only --ignore-scripts` and
`npm audit --omit=dev --package-lock-only --ignore-scripts`.
The blanket test command installs Electron and includes application/installer
lifecycle tests, so it must not be used when native windows are prohibited.
The TypeScript 7 Node launcher and electron-builder's process helper do not
set windowsHide; invoke the compiler binary with windowsHide explicitly and
defer native packaging unless all child process paths are verified windowless.

Verification on Node 24.22.2: clean script-free install, TypeScript build and
asset copies, 16 tests (build dependencies and comments), full and production
lockfile audits (both zero findings), and the production-collector check above.
Windows unpacked/NSIS configuration is unchanged. Native packaging and
application/installer lifecycle tests were not run under the no-windows rule;
final artifact behavior still requires validation in a suitable environment.
