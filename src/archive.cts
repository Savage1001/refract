const fs = require('node:fs/promises');
const path = require('node:path');
const crypto = require('node:crypto');
const Zip = require('adm-zip');
const { crc32 } = require('adm-zip/util');
function readEntry(entry: { getData(): Buffer; header: { size: number; crc: number } }): Buffer {
  const data = entry.getData();
  if (data.length !== entry.header.size || (crc32(data) >>> 0) !== (entry.header.crc >>> 0)) {
    throw new Error('Invalid file data in project archive');
  }
  return data;
}
const MAX_ARCHIVE_BYTES=500*1024*1024;

// TeX treats an unescaped percent as a comment, but \% is document text.
function uncomment(line: string): string {
  for (let i = 0; i < line.length; i++) {
    if (line[i] !== '%') continue;
    let slashes = 0;
    for (let j = i - 1; j >= 0 && line[j] === '\\'; j--) slashes++;
    if (slashes % 2 === 0) return line.slice(0, i);
  }
  return line;
}

async function exportProject(store, id, output) {
  await store.destinationDir(id);
  const zip = new Zip();
  const addDirectories = nodes => {
    for (const node of nodes) {
      if (node.children) { zip.addFile(node.path + '/', Buffer.alloc(0)); addDirectories(node.children); }
    }
  };
  addDirectories(await store.tree(id));
  for (const name of await store.files(id)) zip.addFile(name, await fs.readFile(await store.existingFile(id, name)));
  await fs.writeFile(output, zip.toBuffer());
}
async function importProject(store, archive, title, selectRoot?: (candidates: string[]) => Promise<string | string[] | null>) {
  const info=await fs.stat(archive);
  if(!info.isFile() || info.size>MAX_ARCHIVE_BYTES) throw new Error('Project archive is too large or is not a file');
  const zip = new Zip(archive), allEntries = zip.getEntries(), entries = allEntries.filter(e => !e.isDirectory);
  if (!entries.length || allEntries.length > 10000) throw new Error('Archive has no files or too many files');
  let total = 0;
  // Windows normalizes trailing dots and spaces and reserves device names.
  const paths = new Map<string, { name: string; directory: boolean }>();
  for (const entry of allEntries) {
    total += entry.header.size;
    if (total > MAX_ARCHIVE_BYTES) throw new Error('Project archive is too large');
    if (!entry.isDirectory && entry.header.method===0 && entry.header.compressedSize!==entry.header.size) throw new Error('Invalid stored file size in project archive');
    const components = entry.entryName.replace(/\/$/, '').split('/');
    if (entry.entryName.startsWith('/') || entry.entryName.includes('\\') || components.some(part => !part || part === '..' || part === '.' || part.toLowerCase().startsWith('.refract') || /[<>:"|?*\x00-\x1f]/.test(part) || /[. ]$/.test(part) || /^(con|prn|aux|nul|com[1-9¹²³]|lpt[1-9¹²³])(?:\.|$)/i.test(part))) throw new Error('Invalid path in project archive');
    for (let i = 0; i < components.length; i++) {
      const name = components.slice(0, i + 1).join('/'), key = name.toLowerCase();
      const directory = i < components.length - 1 || entry.isDirectory;
      const previous = paths.get(key);
      if (previous && (previous.name !== name || previous.directory !== directory || !directory)) throw new Error('Archive contains conflicting paths');
      paths.set(key, { name, directory });
    }
  }
  const parts = entries.map(e => e.entryName.split('/'));
  const root = parts.every(p => p.length > 1 && p[0] === parts[0][0]) && allEntries.every(e => e.entryName === parts[0][0] + '/' || e.entryName.startsWith(parts[0][0] + '/')) ? parts[0][0] + '/' : '';
  const names = entries.map(e => e.entryName.slice(root.length));
  if (new Set(names.map(n => n.toLowerCase())).size !== names.length) throw new Error('Archive contains duplicate file names');
  const texFiles = names.filter(name => /\.tex$/i.test(name));
  const candidates = texFiles.filter(name => {
    const source = readEntry(entries[names.indexOf(name)]).toString('utf8');
    return source.split(/\r?\n/).some(line => /\\documentclass(?:\s*\[[^\]]*\])?\s*\{/.test(uncomment(line)));
  });
  const mainFile = names.find(name => name.toLowerCase() === 'main.tex');
  const choices = [...new Set([...candidates, ...(mainFile ? [mainFile] : [])])];
  let rootFile = choices.length === 1 ? choices[0] : null;
  if (choices.length > 1 && selectRoot) {
    rootFile = await selectRoot(choices);
    if (rootFile === null) return null;
    if (Array.isArray(rootFile) ? !rootFile.length || rootFile.some(file => !choices.includes(file)) || new Set(rootFile).size !== rootFile.length : !choices.includes(rootFile)) throw new Error('Invalid LaTeX root selection');
  }
  if (!rootFile) throw new Error(choices.length > 1 ? 'Project contains multiple LaTeX root files; select a root file' : 'Project does not contain a LaTeX root file');
  const roots = Array.isArray(rootFile) ? rootFile : [rootFile];
  // Distinct top-level folders identify projects in a multi-project export.
  const folders = choices.map(file => file.includes('/') ? file.split('/')[0] : '');
  const separated = choices.length > 1 && folders.every(Boolean) && new Set(folders).size === folders.length;
  if (roots.length > 1 && !separated) throw new Error('Cannot separate projects in this archive: each root document must be in its own top-level folder');
  await store.init();
  const created: string[] = [];
  let first;
  try {
  for (const selectedRoot of roots) {
  const prefix = separated ? selectedRoot.split('/')[0] + '/' : '';
  const id = crypto.randomUUID(), dir = store.projectDir(id);
  await fs.mkdir(dir);
  created.push(dir);
    for (const entry of allEntries.filter(e => e.isDirectory)) {
      const relative = entry.entryName.slice(root.length);
      if (prefix && !relative.startsWith(prefix)) continue;
      const name = relative.slice(prefix.length).replace(/\/$/, '');
      if (name) await fs.mkdir(store.filePath(id, name), { recursive: true });
    }
    for (let i = 0; i < entries.length; i++) {
      if (prefix && !names[i].startsWith(prefix)) continue;
      const dest = store.filePath(id, names[i].slice(prefix.length));
      await fs.mkdir(path.dirname(dest), { recursive: true });
      await fs.writeFile(dest, readEntry(entries[i]), { flag: 'wx' });
    }
    const name = String(title).trim() || 'Imported Project';
    const meta = { id, name: separated ? prefix.slice(0, -1) : name, created: new Date().toISOString(), rootFile: selectedRoot.slice(prefix.length) };
    await fs.writeFile(path.join(dir, '.refract.json'), JSON.stringify(meta, null, 2));
    first ??= meta;
  }
  return first;
  } catch (error) {
    for (const dir of created) await fs.rm(dir, { recursive: true, force: true });
    throw error;
  }
}
module.exports = { importProject, exportProject };
