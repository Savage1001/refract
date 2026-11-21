import fs = require('node:fs/promises');
import path = require('node:path');
import sources = require('./source-cache.cjs');
const {mapLimited, signature} = sources;
interface ImageStore {
  destinationDir(id: string, relative?: string, create?: boolean): Promise<string>;
  files(id: string): Promise<string[]>;
  existingFile(id: string, name: string): Promise<string>;
  write(id: string, name: string, text: string): Promise<void>;
}

// Cache only files whose contents had no Refract attachment references. Each
// build still validates the path and checks for edits and file replacements.
const scannedFiles = new WeakMap<ImageStore, Map<string, string>>();
const scanConcurrency = 8;

// Keep offsets intact while hiding comments and literal examples from the matcher.
function documentCode(text: string): string {
  return text.replace(/\\begin\{(verbatim\*?|lstlisting|minted)\}[\s\S]*?\\end\{\1\}|\\verb\*?([^\w\s])[\s\S]*?\2|(?<!\\)(?:\\\\)*%[^\r\n]*/g,
    match => match.replace(/[^\r\n]/g, ' '));
}

async function materializeDocumentImages(store: ImageStore, id: string): Promise<void> {
  const root = await store.destinationDir(id);
  let scans = scannedFiles.get(store);
  if (!scans) scannedFiles.set(store, scans = new Map());
  const names = (await store.files(id)).filter(name => name.endsWith('.tex'));
  const scan = async (name: string) => {
    const file = await store.existingFile(id, name);
    const info = await fs.lstat(file, {bigint: true});
    if (!info.isFile() || info.isSymbolicLink()) throw new Error('Only regular project files can be used');
    if (info.nlink !== 1n || info.size > 2n * 1024n * 1024n) return;
    const stamp = signature(info);
    if (scans.get(file) === stamp) return;
    const text = await fs.readFile(file, 'utf8');
    const references: Array<{reference: string; offset: number}> = [];
    if (text.includes('.refract-attachments/') && text.includes('\\includegraphics')) {
      const graphics = /\\includegraphics\*?\s*(?:\[[^\]]*\]\s*)?\{\s*(?:\\detokenize\{([^{}]+)\}|([^{}]+?))\s*\}/g;
      for (const match of documentCode(text).matchAll(graphics)) {
        const reference = (match[1] ?? match[2]).trim();
        // Only literal paths to Refract image attachments are eligible.
        if (/^(?:\.\/)?\.refract-attachments\/[0-9a-f-]{36}\.(?:png|jpe?g|webp)$/.test(reference)) {
          references.push({reference, offset: match.index! + match[0].indexOf(reference)});
        }
      }
    }
    if (references.length) return {name, file, stamp, text, references};
    if (stamp === signature(await fs.lstat(file, {bigint: true}))) {
      if (scans.size >= 2048) scans.delete(scans.keys().next().value!);
      scans.set(file, stamp);
    }
  };
  const materialize = async (document: NonNullable<Awaited<ReturnType<typeof scan>>>) => {
    const {name, file, stamp, text, references} = document;
    if (file !== await store.existingFile(id, name) || stamp !== signature(await fs.lstat(file, {bigint: true}))) {
      throw new Error('Document changed while preparing image attachments; retry compilation');
    }
    const replacements: Array<{start: number; end: number; value: string}> = [];
    for (const {reference, offset} of references) {
      const token = path.posix.basename(reference), sourceDir = path.join(root, '.refract-attachments');
      const dirInfo = await fs.lstat(sourceDir);
      if (!dirInfo.isDirectory() || dirInfo.isSymbolicLink()) throw new Error('Invalid attachment directory');
      const source = path.join(sourceDir, token), sourceInfo = await fs.lstat(source);
      if (!sourceInfo.isFile() || sourceInfo.isSymbolicLink() || sourceInfo.nlink !== 1 || sourceInfo.size > 20 * 1024 * 1024) throw new Error('Invalid document image attachment');
      const destinationDir = await store.destinationDir(id, 'images', true);
      const destination = `images/chat-${token}`, target = path.join(destinationDir, `chat-${token}`);
      try { await fs.copyFile(source, target, fs.constants.COPYFILE_EXCL); }
      catch (error) {
        if (error.code !== 'EEXIST') throw error;
        const existing = await fs.lstat(target);
        if (!existing.isFile() || existing.isSymbolicLink() || existing.nlink !== 1 || existing.size !== sourceInfo.size || !(await fs.readFile(target)).equals(await fs.readFile(source))) throw new Error('Document image destination already exists');
      }
      replacements.push({start: offset, end: offset + reference.length, value: destination});
    }
    let updated = text;
    for (const replacement of replacements.reverse()) updated = updated.slice(0, replacement.start) + replacement.value + updated.slice(replacement.end);
    await store.write(id, name, updated);
  };
  // Bound the number of documents held in memory. Copies and rewrites run in
  // order so documents that share an attachment see a complete image.
  for (let index = 0; index < names.length; index += scanConcurrency) {
    const documents = await mapLimited(names.slice(index, index + scanConcurrency), scan, scanConcurrency);
    for (const document of documents) if (document) await materialize(document);
  }
}

export = {materializeDocumentImages};
