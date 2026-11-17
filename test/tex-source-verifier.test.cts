import type { TestContext } from 'node:test';
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

type Entry = { name: string; sha512: string };
async function fixture(t: TestContext): Promise<{ root: string; entries: Entry[] } | undefined> {
 const project = path.resolve(__dirname, '../..');
 const catalogue = path.resolve(process.env.REFRACT_TEX_BIN || 'missing-tex', '..', '..', 'tlpkg/texlive.tlpdb');
 try { await fs.access(catalogue); } catch { t.skip('Historical TeX catalogue is not available'); return; }
 const root: string = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-source-verifier-'));
 t.after(() => fs.rm(root, { recursive: true, force: true }));
 for (const dir of ['build/scripts', 'docs', 'vendor/TinyTeX/tlpkg', 'archives']) await fs.mkdir(path.join(root, dir), { recursive: true });
 await fs.copyFile(path.join(project, 'build/scripts/verify-tex-sources.cjs'), path.join(root, 'build/scripts/verify-tex-sources.cjs'));
 await fs.copyFile(catalogue, path.join(root, 'vendor/TinyTeX/tlpkg/texlive.tlpdb'));
 const entries: Entry[] = JSON.parse((await fs.readFile(path.join(project, 'docs/tex-source-manifest.json'), 'utf8')).replace(/^\uFEFF/, ''));
 return { root, entries };
}

async function run(root: string, entries: Entry[]): Promise<{ status: number | null; stderr: string }> {
 await fs.writeFile(path.join(root, 'docs/tex-source-manifest.json'), JSON.stringify(entries));
 const result = spawnSync(process.execPath, [path.join(root, 'build/scripts/verify-tex-sources.cjs'), path.join(root, 'archives')], { encoding: 'utf8', windowsHide: true });
 if (result.error) throw result.error;
 return result;
}

test('source verification rejects an incomplete manifest', async (t: TestContext) => {
 const item = await fixture(t); if (!item) return;
 const result = await run(item.root, item.entries.slice(1));
 assert.notEqual(result.status, 0); assert.match(result.stderr, /does not cover/);
});

test('source verification rejects a modified catalogue', async (t: TestContext) => {
 const item = await fixture(t); if (!item) return;
 await fs.appendFile(path.join(item.root, 'vendor/TinyTeX/tlpkg/texlive.tlpdb'), 'changed');
 const result = await run(item.root, item.entries);
 assert.notEqual(result.status, 0); assert.match(result.stderr, /catalogue does not match/);
});

test('source verification rejects a manifest checksum that differs from the catalogue', async (t: TestContext) => {
 const item = await fixture(t); if (!item) return;
 item.entries[0].sha512 = '0'.repeat(128);
 const result = await run(item.root, item.entries);
 assert.notEqual(result.status, 0); assert.match(result.stderr, /Manifest does not match/);
});

test('source verification rejects a corrupted source archive', async (t: TestContext) => {
 const item = await fixture(t); if (!item) return;
 await fs.writeFile(path.join(item.root, 'archives', `${item.entries[0].name}.source.tar.xz`), 'corrupt');
 const result = await run(item.root, item.entries);
 assert.notEqual(result.status, 0); assert.match(result.stderr, /Checksum mismatch/);
});
