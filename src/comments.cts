const fs = require('node:fs/promises');
const path = require('node:path');
const crypto = require('node:crypto');
const normalize = text => text.replace(/\r\n?/g,'\n');
const fingerprint = text => crypto.createHash('sha256').update(text,'utf8').digest('hex');
const makeAnchor = (text, start, end) => ({
 start, end, prefix:text.slice(Math.max(0,start-48),start), suffix:text.slice(end,end+48),
 sourceHash:fingerprint(text), status:'attached'
});
function locateSelection(text, comment) {
 const quote = comment.quote, anchor = comment.anchor;
 if (!quote || typeof quote !== 'string' || anchor?.status === 'orphaned') return null;
 if (anchor && anchor.sourceHash === fingerprint(text) && text.slice(anchor.start,anchor.end) === quote) return {start:anchor.start,end:anchor.end};
 const candidates = [];
 for (let at=text.indexOf(quote); at !== -1; at=text.indexOf(quote,at+1)) {
  if (candidates.length >= 10000) return null;
  candidates.push(at);
 }
 if (!anchor) return candidates.length === 1 ? {start:candidates[0],end:candidates[0]+quote.length} : null;
 let best=-1, chosen=-1, tied=false;
 for (const at of candidates) {
  let left=0,right=0;
  while (left < anchor.prefix.length && left < at && anchor.prefix[anchor.prefix.length-1-left] === text[at-1-left]) left++;
  while (right < anchor.suffix.length && at+quote.length+right < text.length && anchor.suffix[right] === text[at+quote.length+right]) right++;
  const score=left+right;
  if (score>best) { best=score; chosen=at; tied=false; } else if (score===best) tied=true;
 }
 const minimum=Math.min(8,anchor.prefix.length+anchor.suffix.length);
 return chosen >= 0 && !tied && minimum > 0 && best >= minimum ? {start:chosen,end:chosen+quote.length} : null;
}
class Comments {
 store: any;
 constructor(store) { this.store = store; }
 file(id) { return path.join(this.store.projectDir(id), '.refract-comments.json'); }
 async checkFile(file:string,allowMissing=false):Promise<void> {
  try {const info=await fs.lstat(file);if(info.isSymbolicLink() || !info.isFile() || info.nlink>1)throw new Error('Invalid comments file');}
  catch(error) {if(error.code!=='ENOENT' || !allowMissing)throw error;}
 }
 async list(id) { await this.store.destinationDir(id);const file=this.file(id);try {await this.checkFile(file);return JSON.parse(await fs.readFile(file,'utf8'));} catch(e) { if(e.code === 'ENOENT') return []; throw e; } }
 async save(id, comments) {
  await this.store.destinationDir(id);
  const file=this.file(id), temp=`${file}.${crypto.randomUUID()}.tmp`;
  await this.checkFile(file,true);
  const handle=await fs.open(temp,'wx');
  try { await handle.writeFile(JSON.stringify(comments),'utf8');await handle.close();await fs.rename(temp,file); }
  catch (error) { await handle.close().catch(()=>{});await this.store.destinationDir(id).then(()=>fs.unlink(temp)).catch(()=>{});throw error; }
 }
 async add(id, file, quote, text, selection?: {start:number; end:number}) {
  this.store.filePath(id,file);
  if (typeof text !== 'string' || !text.trim()) throw new Error('Comment is empty');
  if (typeof quote !== 'string' || quote.length > 10000) throw new Error('Invalid selection');
  let anchor = null;
  if (quote) {
   const source=normalize(await this.store.read(id,file));
   let start=selection?.start, end=selection?.end;
   if (selection) {
    if (!Number.isInteger(start) || !Number.isInteger(end) || start < 0 || end <= start || source.slice(start,end) !== quote) throw new Error('The selected text has changed; select it again');
   } else {
    start=source.indexOf(quote); end=start+quote.length;
    if (start < 0 || source.indexOf(quote,start+1) !== -1) { start=-1; end=-1; }
   }
   if (start >= 0) anchor=makeAnchor(source,start,end);
  }
  const comments=await this.list(id), item={id:crypto.randomUUID(),file,quote,text:text.trim(),created:new Date().toISOString(),resolved:false,anchor};
  comments.push(item); await this.save(id,comments); return item;
 }
 async locate(id, commentId) {
  const item=(await this.list(id)).find(c=>c.id===commentId);
  if (!item) throw new Error('Comment not found');
  try { return locateSelection(normalize(await this.store.read(id,item.file)),item); }
  catch (error) { if (error.code==='ENOENT') return null; throw error; }
 }
 async planEdit(id, file, before, after) {
  if (before !== null) before=normalize(before);
  if (after !== null) after=normalize(after);
  if (before === after) return null;
  const comments=await this.list(id); let changed=false;
  if (before === null || after === null) {
   for (const item of comments) if (item.file===file && item.anchor && item.anchor.status!=='orphaned') { item.anchor.status='orphaned'; changed=true; }
   return changed ? comments : null;
  }
  let prefix=0; while(prefix<before.length && prefix<after.length && before[prefix]===after[prefix]) prefix++;
  let suffix=0; while(suffix<before.length-prefix && suffix<after.length-prefix && before[before.length-1-suffix]===after[after.length-1-suffix]) suffix++;
  const oldEnd=before.length-suffix, delta=after.length-before.length;
  for (const item of comments) {
   if (item.file!==file || !item.anchor || item.anchor.status==='orphaned') continue;
   const position=locateSelection(before,item);
   if (!position) { item.anchor.status='orphaned'; changed=true; continue; }
   let {start,end}=position;
   if (end <= prefix) { /* edit follows the selection */ }
   else if (start >= oldEnd) { start+=delta; end+=delta; }
   else if (prefix >= start && oldEnd <= end) { end+=delta; }
   else {
    // Multiple separate edits can span an unchanged selection. Reattach only
    // when the number of identical quotes is unchanged and context identifies
    // one unambiguous occurrence; a deleted duplicate must not steal a comment.
    const count = (text: string, quote: string) => {
     let result = 0, offset = 0;
     while (result <= 10000) { const found = text.indexOf(quote, offset); if (found < 0) break; result++; offset = found + 1; }
     return result;
    };
    const beforeCount = count(before,item.quote), afterCount = count(after,item.quote);
    const located = beforeCount > 0 && beforeCount <= 10000 && beforeCount === afterCount ? locateSelection(after,item) : null;
    const left = item.anchor.prefix.slice(-8), right = item.anchor.suffix.slice(0,8);
    if (!located || after.slice(Math.max(0,located.start-left.length),located.start)!==left || after.slice(located.end,located.end+right.length)!==right) { item.anchor.status='orphaned'; changed=true; continue; }
    start = located.start; end = located.end;
   }
   if (end <= start) { item.anchor.status='orphaned'; changed=true; continue; }
   item.quote=after.slice(start,end); item.anchor=makeAnchor(after,start,end); changed=true;
  }
  return changed ? comments : null;
 }
 async renameFile(id, oldName, newName) {
  this.store.filePath(id,newName); const comments=await this.list(id); let changed=false;
  for (const item of comments) if (typeof item.file === 'string' && item.file.replaceAll('\\','/').toLowerCase() === oldName.replaceAll('\\','/').toLowerCase()) {item.file=newName;changed=true;}
  if (changed) await this.save(id,comments);
 }
 async moveFolder(id, oldName, newName) {
  this.store.filePath(id,newName); const comments=await this.list(id); let changed=false;
  const prefix=oldName.replaceAll('\\','/')+'/';
  for(const item of comments) if(typeof item.file==='string' && item.file.replaceAll('\\','/').toLowerCase().startsWith(prefix.toLowerCase())) {
   item.file=newName+item.file.replaceAll('\\','/').slice(prefix.length-1);changed=true;
  }
  if(changed) await this.save(id,comments);
 }
 async resolve(id, commentId, resolved) {
  const comments=await this.list(id), item=comments.find(c=>c.id===commentId);
  if (!item) throw new Error('Comment not found'); item.resolved=!!resolved; await this.save(id,comments); return item;
 }
 async reply(id, commentId, text) {
  if (typeof text !== 'string' || !text.trim()) throw new Error('Reply is empty');
  const comments=await this.list(id), item=comments.find(c=>c.id===commentId);
  if (!item) throw new Error('Comment not found');
  const reply={id:crypto.randomUUID(),text:text.trim(),created:new Date().toISOString()};
  if (!Array.isArray(item.replies)) item.replies=[];
  item.replies.push(reply); await this.save(id,comments); return reply;
 }
}
module.exports = { Comments };
