import fs = require('node:fs/promises');
import path = require('node:path');
interface ImageStore {
  destinationDir(id: string, relative?: string, create?: boolean): Promise<string>;
  files(id: string): Promise<string[]>;
  existingFile(id: string, name: string): Promise<string>;
  write(id: string, name: string, text: string): Promise<void>;
}

// Keep offsets intact while hiding comments and literal examples from the matcher.
function documentCode(text: string): string {
  return text.replace(/\\begin\{(verbatim\*?|lstlisting|minted)\}[\s\S]*?\\end\{\1\}|\\verb\*?([^\w\s])[\s\S]*?\2|(?<!\\)(?:\\\\)*%[^\r\n]*/g,
    match => match.replace(/[^\r\n]/g, ' '));
}

async function materializeDocumentImages(store: ImageStore, id: string): Promise<void> {
  const root = await store.destinationDir(id);
  for (const name of await store.files(id) as string[]) {
    if (!name.endsWith('.tex')) continue;
    const file = await store.existingFile(id, name), info = await fs.lstat(file);
    if (info.nlink !== 1 || info.size > 2 * 1024 * 1024) continue;
    const text = await fs.readFile(file, 'utf8'), code = documentCode(text);
    const replacements: Array<{start: number; end: number; value: string}> = [];
    const graphics = /\\includegraphics\*?\s*(?:\[[^\]]*\]\s*)?\{\s*(?:\\detokenize\{([^{}]+)\}|([^{}]+?))\s*\}/g;
    for (const match of code.matchAll(graphics)) {
      const reference = (match[1] ?? match[2]).trim();
      // Only literal paths to Refract image attachments are eligible. Never expose other private files.
      if (!/^(?:\.\/)?\.refract-attachments\/[0-9a-f-]{36}\.(?:png|jpe?g|webp)$/.test(reference)) continue;
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
      const offset = match.index! + match[0].indexOf(reference);
      replacements.push({start: offset, end: offset + reference.length, value: destination});
    }
    if (replacements.length) {
      let updated = text;
      for (const replacement of replacements.reverse()) updated = updated.slice(0, replacement.start) + replacement.value + updated.slice(replacement.end);
      await store.write(id, name, updated);
    }
  }
}

export = {materializeDocumentImages};
