const fs = require('node:fs/promises');
const path = require('node:path');
const MAX_PREVIEW_BYTES = 32 * 1024 * 1024;
const imageTypes = {'.bmp':'image/bmp','.png':'image/png','.jpg':'image/jpeg','.jpeg':'image/jpeg','.gif':'image/gif','.webp':'image/webp','.svg':'image/svg+xml'};
const binaryExtensions = new Set(['.bin','.sqlite','.sqlite3','.db','.class','.wasm','.npy','.pkl','.rds','.zip','.gz','.tar','.7z','.rar','.exe','.dll','.so','.dylib','.woff','.woff2','.ttf','.otf','.eot','.docx','.xlsx','.pptx','.mp3','.mp4','.mov','.avi','.eps']);

async function inspectFile(store, id, name) {
 const root=store.projectDir(id), target=store.filePath(id,name);
 // Project trees do not expose links. Check again here so a crafted IPC request cannot follow one.
 if ((await fs.lstat(root)).isSymbolicLink()) throw new Error('Linked projects cannot be previewed');
 const parts=name.split(/[\\/]/); let cursor=root;
 for (const part of parts) {
  cursor=path.join(cursor,part);
  const stat=await fs.lstat(cursor);
  if (stat.isSymbolicLink()) throw new Error('Files in linked folders cannot be previewed');
 }
 const info=await fs.stat(target);
 if (!info.isFile()) throw new Error('Only files can be previewed');
 if (info.size > MAX_PREVIEW_BYTES) return {kind:'unsupported',reason:'This file is too large to preview.'};
 const handle=await fs.open(target,'r');
 let bytes;
 try {
  const chunks: Buffer[]=[]; let total=0;
  while (total <= MAX_PREVIEW_BYTES) {
   const chunk=Buffer.allocUnsafe(Math.min(64 * 1024, MAX_PREVIEW_BYTES + 1 - total));
   const {bytesRead}=await handle.read(chunk,0,chunk.length,null);
   if (!bytesRead) break;
   chunks.push(chunk.subarray(0,bytesRead)); total+=bytesRead;
  }
  if (total > MAX_PREVIEW_BYTES) return {kind:'unsupported',reason:'This file is too large to preview.'};
  bytes=Buffer.concat(chunks,total);
 } finally { await handle.close(); }
 const extension=path.extname(name).toLowerCase();
 if (extension === '.pdf' || bytes.subarray(0,5).equals(Buffer.from('%PDF-'))) return {kind:'pdf',data:bytes.toString('base64')};
 if (imageTypes[extension]) return {kind:'image',data:`data:${imageTypes[extension]};base64,${bytes.toString('base64')}`};
 const magic = bytes.subarray(0,4);
 const zipMagic = magic.length === 4 && magic[0] === 0x50 && magic[1] === 0x4b && [[3,4],[5,6],[7,8]].some(pair => magic[2] === pair[0] && magic[3] === pair[1]);
 if (binaryExtensions.has(extension) || bytes.includes(0) || magic.equals(Buffer.from([0x7f,0x45,0x4c,0x46])) || bytes.subarray(0,2).equals(Buffer.from('MZ')) || zipMagic) return {kind:'unsupported',reason:'This file cannot be previewed as text.'};
 try {
  const text=new TextDecoder('utf-8',{fatal:true,ignoreBOM:true}).decode(bytes);
  if (/[\u0000-\u0008\u000b\u000e-\u001f\u007f]/.test(text)) return {kind:'unsupported',reason:'This file cannot be previewed as text.'};
  return {kind:'text',text};
 } catch { return {kind:'unsupported',reason:'This file cannot be previewed as text.'}; }
}
module.exports={inspectFile,MAX_PREVIEW_BYTES};
