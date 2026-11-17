import crypto = require('node:crypto');
import fs = require('node:fs');
import promises = require('node:fs/promises');
import path = require('node:path');

type SourceEntry = { name: string; sha512: string };

async function main(): Promise<void> {
 const root = path.resolve(__dirname, '../..');
 const directory = path.resolve(process.argv[2] ?? path.join(root, 'vendor/refract-tex-source-audit/texlive-sources'));
 const text = await promises.readFile(path.join(root, 'docs/tex-source-manifest.json'), 'utf8');
 const data: unknown = JSON.parse(text.replace(/^\uFEFF/, ''));
 if (!Array.isArray(data) || data.length === 0) throw new Error('Invalid or empty source manifest');
 const catalogueBytes = await promises.readFile(path.join(root, 'vendor/TinyTeX/tlpkg/texlive.tlpdb'));
 const catalogueHash = crypto.createHash('sha256').update(catalogueBytes).digest('hex');
 if (catalogueHash !== '466f964737da319f36a140bfa3a82c578ea6a5b45c791a025c0a9e0d7c5e77da') throw new Error('TeX catalogue does not match the pinned TinyTeX archive');
 const expected = new Map<string, string>();
 for (const block of catalogueBytes.toString('utf8').split(/\r?\n\r?\n/)) {
  const name = /^name (.+)\r?$/m.exec(block)?.[1];
  const sha512 = /^srccontainerchecksum ([a-f0-9]{128})\r?$/m.exec(block)?.[1];
  if (sha512) {
   if (!name || expected.has(name)) throw new Error('Invalid or duplicate TeX catalogue entry');
   expected.set(name, sha512);
  }
 }
 if (!expected.size || data.length !== expected.size) throw new Error('Source manifest does not cover the pinned TeX catalogue');
 const names = new Set<string>();
 for (const item of data as unknown[]) {
  if (!item || typeof item !== 'object') throw new Error('Invalid source entry');
  const { name, sha512 } = item as Partial<SourceEntry>;
  if (typeof name !== 'string' || !/^[a-zA-Z0-9._+-]+$/.test(name) || name === '.' || name === '..' || names.has(name)) throw new Error('Invalid or duplicate source name');
  if (typeof sha512 !== 'string' || !/^[a-f0-9]{128}$/.test(sha512)) throw new Error(`Invalid checksum for ${name}`);
  if (expected.get(name) !== sha512) throw new Error(`Manifest does not match the pinned TeX catalogue for ${name}`);
  names.add(name);
  const hash = crypto.createHash('sha512');
  for await (const chunk of fs.createReadStream(path.join(directory, `${name}.source.tar.xz`))) hash.update(chunk);
  if (hash.digest('hex') !== sha512) throw new Error(`Checksum mismatch for ${name}`);
 }
 process.stdout.write(`Verified ${names.size} TeX source archives.\n`);
}

main().catch((error: unknown) => {
 process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
 process.exitCode = 1;
});
