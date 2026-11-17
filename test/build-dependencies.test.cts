const { test } = require('node:test') as typeof import('node:test');
const assert: typeof import('node:assert/strict') = require('node:assert/strict');
const { createRequire } = require('node:module') as typeof import('node:module');
const path = require('node:path') as typeof import('node:path');
const { statSync } = require('node:fs') as typeof import('node:fs');
const { FileMatcher } = require('app-builder-lib/out/fileMatcher') as typeof import('app-builder-lib/out/fileMatcher');

const root = path.resolve(__dirname, '../..');

test('legacy glob consumers retain their compatible minimatch API', () => {
 const glob = require('glob') as { sync(pattern: string, options: { cwd: string }): string[] };
 assert.ok(glob.sync('*.{json,md}', { cwd: root }).includes('package.json'));
});

test('build consumers resolve patched minimatch with its matching brace dependency', () => {
 for (const consumer of ['app-builder-lib', '@electron/universal']) {
  const consumerRequire = createRequire(require.resolve(consumer));
  const version: string = consumerRequire('minimatch/package.json').version;
  assert.match(version, /^11\./);
  assert.ok((require('semver') as { gte(a: string, b: string): boolean }).gte(version, '11.0.3'));
  const minimatchRequire = createRequire(consumerRequire.resolve('minimatch'));
  assert.equal(typeof minimatchRequire('brace-expansion').expand, 'function');
  const { minimatch } = consumerRequire('minimatch') as {
   minimatch(file: string, pattern: string, options: { matchBase: boolean }): boolean;
  };
  // @electron/universal uses this API to allow architecture-specific files.
  assert.equal(minimatch('nested/arm64.node', '*.{node,dylib}', { matchBase: true }), true);
  assert.equal(minimatch('nested/secret.txt', '*.{node,dylib}', { matchBase: true }), false);
 }
});

test('builder preserves application and TeX-free distribution filters', () => {
 const config = require('../../package.json') as { build: { files: string[]; extraResources: { filter: string[] }[] } };
 const fileStat = statSync(path.join(root, 'package.json'));
 const dirStat = statSync(root);
 const accepts = (patterns: string[], relative: string, directory = false): boolean =>
  new FileMatcher(root, path.join(root, 'dist'), value => value, patterns)
   .createFilter()(path.join(root, relative), directory ? dirStat : fileStat);
 for (const file of ['build/main.cjs', 'build/src/compiler.cjs', 'build/fonts/inter.woff2', 'node_modules/zod/index.cjs', 'LICENSE', 'package.json'])
  assert.equal(accepts(config.build.files, file), true, file);
 for (const file of ['build/.hidden', 'build/test/fixture.exe', 'vendor/TinyTeX/bin/windows/pdflatex.exe', 'node_modules/@napi-rs/canvas/index.js', 'node_modules/@napi-rs/canvas-win32-x64-msvc/canvas.node', 'src/main.cts'])
  assert.equal(accepts(config.build.files, file), false, file);
 assert.deepEqual(config.build.extraResources, []);
 for (const file of ['assets/logo.png', 'assets/icons/logo.ico'])
  assert.equal(accepts(['assets/**/*.{png,ico}', '!**/private/**'], file), true);
 assert.equal(accepts(['assets/**/*.{png,ico}', '!**/private/**'], 'assets/private/logo.png'), false);
 assert.equal(accepts(['assets/**/logo.png'], 'assets/icons', true), true);
 assert.equal(accepts(['assets'], 'assets/icons/logo.png'), true);
 assert.equal(accepts(['**/*.@(png|ico)'], 'assets/logo.ico'), true);
});
