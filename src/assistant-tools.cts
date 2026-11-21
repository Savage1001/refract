import fs = require('node:fs/promises');
import crypto = require('node:crypto');
import http = require('node:http');
import mcpSdk = require('@modelcontextprotocol/sdk/server/mcp.js');
import transportSdk = require('@modelcontextprotocol/sdk/server/streamableHttp.js');
import zod = require('zod');
import toolchain = require('./tex-toolchain.cjs');
const {McpServer} = mcpSdk, {StreamableHTTPServerTransport} = transportSdk, {z} = zod;

export interface ToolConnection { url: string; token: string }
type LatexEngine = typeof toolchain.latexEngines[number];
interface WorkspaceStore {
  existingFile(id: string, name: string): Promise<string>;
  destinationDir(id: string): Promise<string>;
  files(id: string): Promise<string[]>;
  write(id: string, name: string, content: string): Promise<void>;
}
interface ToolOptions {
  performance?: InstanceType<typeof import('./assistant-performance.cjs').AssistantPerformance>;
  history?: () => Promise<{messages:Array<{role:string;text?:string;attachments?:Array<{token:string;name:string}>}>}>;
  initialEngine?: LatexEngine;
  setEngine?: (engine: LatexEngine) => Promise<{engine: LatexEngine; previousEngine: LatexEngine | null; changed: boolean}>;
}
const MAX_BYTES = 2 * 1024 * 1024;
const fileName = z.string().min(1).max(1024).refine(name => !name.split(/[\\/]/).some(part => part.startsWith('.')), 'Private project files are unavailable');
const hash = (text: string) => crypto.createHash('sha256').update(text).digest('hex');
const result = (value: unknown) => ({content: [{type: 'text' as const, text: JSON.stringify(value)}]});
interface Replacement { oldText: string; newText: string }
const replacement = {oldText: z.string().min(1).max(MAX_BYTES), newText: z.string().max(MAX_BYTES)};
const editSchema = {
  path: fileName,
  sha256: z.string().regex(/^[0-9a-f]{64}$/).optional(),
  ...replacement,
  additional: z.array(z.object(replacement)).max(7).optional(),
};
const engineSelectionPolicy =
  'Choose the fastest LaTeX engine that fully meets the user, template, package, font, language and output requirements. ' +
  'For a speed request, check the document and template first. If they are compatible with pdfLaTeX and currently use XeLaTeX or LuaLaTeX, switch to pdfLaTeX and build, unless project timings favor another engine. ' +
  'Refract’s small-document Windows benchmarks found pdfLaTeX faster than both alternatives. Use project-specific measurements when available. ' +
  'When the document needs XeLaTeX or LuaLaTeX, choose the fastest compatible option based on its requirements and any existing timings. ' +
  'For a routine build, keep a compatible current engine. ' +
  'After switching, build and check for compatibility warnings and required output features. If the new engine proves unsuitable, restore the previous engine reported by the tool when one is available. Tell the user which engine you selected.';

// Every operation is scoped to one project and one assistant turn. No arbitrary
// commands are exposed here; those remain in the provider's normal sandbox.
function createWorkspaceTools(store: WorkspaceStore, id: string) {
  let queue: Promise<unknown> = Promise.resolve();
  const serialize = <T,>(work: () => Promise<T>): Promise<T> => {
    const job = queue.catch(() => {}).then(work); queue = job; return job;
  };
  const read = async (name: string) => {
    fileName.parse(name);
    const file = await store.existingFile(id, name);
    const handle = await fs.open(file, 'r');
    try {
      const info = await handle.stat();
      if (!info.isFile() || info.nlink > 1 || info.size > MAX_BYTES) throw new Error('Expected a regular text file no larger than 2 MiB');
      const buffer = Buffer.alloc(info.size + 1);
      let length = 0;
      while (length < buffer.length) {
        const {bytesRead} = await handle.read(buffer, length, buffer.length - length, null);
        if (!bytesRead) break; length += bytesRead;
      }
      const after = await handle.stat();
      if (length !== info.size || after.size !== info.size || after.mtimeMs !== info.mtimeMs) throw new Error('File changed while reading; retry');
      const bytes = buffer.subarray(0, length);
      if (bytes.includes(0)) throw new Error('Binary files cannot be edited');
      const content = new TextDecoder('utf-8', {fatal: true, ignoreBOM: true}).decode(bytes);
      return {path: name, sha256: hash(content), content};
    } finally { await handle.close(); }
  };
  return {
    read: (name: string) => serialize(() => read(name)),
    readMany: (names:string[]) => serialize(async()=>Promise.all(names.map(async name=>{try{return await read(name);}catch(error){return {path:name,error:String(error.message)};}}))),
    list: () => serialize(async () => {
      await store.destinationDir(id); const files = await store.files(id);
      return {files: files.slice(0, 10000), ...(files.length > 10000 ? {truncated: true} : {})};
    }),
    edit: (name: string, expectedHash: string | undefined, oldText: string, newText: string, additional: Replacement[] = []) => serialize(async () => {
      const before = await read(name);
      if (expectedHash !== undefined && before.sha256 !== expectedHash) throw new Error('File changed since it was read; read again before editing');
      const crlf = before.content.includes('\r\n') && !/(^|[^\r])\n/.test(before.content);
      const edits = [{oldText, newText}, ...additional].map(edit => {
        if (crlf) {
          edit = {oldText: edit.oldText.replace(/\r?\n/g, '\r\n'), newText: edit.newText.replace(/\r?\n/g, '\r\n')};
        }
        if (!edit.oldText || edit.oldText === edit.newText) throw new Error('Supply a nonempty exact match and a different replacement');
        if (edit.newText.includes('\0') || Buffer.from(edit.newText).toString('utf8') !== edit.newText) throw new Error('Replacement must be valid Unicode text without NUL bytes');
        const start = before.content.indexOf(edit.oldText);
        if (start < 0 || before.content.indexOf(edit.oldText, start + 1) >= 0) throw new Error('Expected exactly one match; read the file and supply more context');
        return {start, end: start + edit.oldText.length, text: edit.newText};
      }).sort((a, b) => a.start - b.start);
      let end = 0;
      const parts: string[] = [];
      for (const edit of edits) {
        if (edit.start < end) throw new Error('Replacements overlap; supply disjoint passages');
        parts.push(before.content.slice(end, edit.start), edit.text);
        end = edit.end;
      }
      parts.push(before.content.slice(end));
      const content = parts.join('');
      if (Buffer.byteLength(content) > MAX_BYTES) throw new Error('Edited file exceeds 2 MiB');
      if (Buffer.from(content).toString('utf8') !== content) throw new Error('Edit must preserve valid Unicode text');
      await store.write(id, name, content);
      const after = await read(name);
      if (after.content !== content) throw new Error('Edit verification failed; read the file again');
      return {path: name, applied: true, sha256: after.sha256, bytes: Buffer.byteLength(content)};
    }),
    drain: async () => { await queue.catch(() => {}); },
    exclusive: serialize,
  };
}

async function startAssistantTools(store: WorkspaceStore, id: string, build: (force?:boolean) => Promise<unknown>, lookup: (name: string) => Promise<unknown>, options:ToolOptions = {}) {
  const workspace = createWorkspaceTools(store, id);
  const token = crypto.randomBytes(32).toString('hex');
  let closing = false;
  let successfulBuild = false;
  let closeJob: Promise<void> | undefined;
  let busyBuild = false;
  const active = new Set<Promise<void>>();
  const clients = new Set<mcpSdk.McpServer>();
  const server = http.createServer((req, res) => {
    if (closing || req.headers.authorization !== `Bearer ${token}` || req.headers.origin) { res.writeHead(403).end(); return; }
    if (req.url !== '/mcp') { res.writeHead(404).end(); return; }
    if (req.method !== 'POST') { res.writeHead(405, {Allow: 'POST'}).end(); return; }
    const job = (async () => {
      let size = 0; const chunks: Buffer[] = [];
      for await (const chunk of req) {
        size += chunk.length;
        if (size > 6 * MAX_BYTES) { res.writeHead(413).end(); return; }
        chunks.push(chunk);
      }
      let body: unknown;
      try { body = JSON.parse(Buffer.concat(chunks).toString('utf8')); }
      catch { res.writeHead(400).end(); return; }
      const mcp = new McpServer({name: 'refract', version: '1.0.0'});
      const transport = new StreamableHTTPServerTransport({sessionIdGenerator: undefined, enableJsonResponse: true});
      clients.add(mcp);
      const track = async <T,>(work:()=>Promise<T>)=>{
        options.performance?.count('toolCalls');
        return options.performance ? options.performance.measure('tool',work) : work();
      };
      const excerpt = (file:{path:string;sha256:string;content:string},offset=0,limit=16000)=>({path:file.path,sha256:file.sha256,offset,totalCharacters:file.content.length,nextOffset:offset+limit<file.content.length ? offset+limit : null,content:file.content.slice(offset,offset+limit)});
      const range = {offset:z.number().int().min(0).max(MAX_BYTES).default(0),limit:z.number().int().min(1).max(65536).default(16000)};
      mcp.registerTool('project_read', {
        description: 'Read project text and its full-file SHA-256. Follow nextOffset for more text. With find, return context around an exact match starting at offset.',
        inputSchema: {path: fileName, ...range, find: z.string().min(1).max(256).optional()},
      }, async ({path,offset,limit,find}) => result(await track(async () => {
        const file = await workspace.read(path);
        if (find === undefined) return excerpt(file, offset, limit);
        const matchOffset = file.content.indexOf(find, offset);
        if (matchOffset < 0) return {...excerpt(file, offset, 0), nextOffset: null, matchOffset: null};
        const before = Math.min(120, Math.floor((Math.max(limit, find.length) - find.length) / 2));
        return {...excerpt(file, Math.max(0, matchOffset - before), Math.min(limit, Math.max(600, find.length))), matchOffset,
          nextOffset: file.content.indexOf(find, matchOffset + 1) < 0 ? null : matchOffset + 1};
      })));
      mcp.registerTool('project_read_many', {description:'Read up to 8 independent files in one call. Each excerpt defaults to 8000 characters; errors are reported per file. Prefer this to separate calls.',inputSchema:{paths:z.array(fileName).min(1).max(8),limit:z.number().int().min(1).max(8000).default(8000)}},async({paths,limit})=>result(await track(async()=>(await workspace.readMany(paths)).map(file=>'error' in file ? file : excerpt(file,0,limit)))));
      mcp.registerTool('project_list', {description: 'List project files directly, without starting a shell.', inputSchema: {}}, async () => result(await track(()=>workspace.list())));
      mcp.registerTool('project_edit', {
        description: 'Replace unique exact text and verify the save. When path and old text are known, edit directly; if you read the file, supply its SHA-256. Use additional for disjoint replacements in the same file, validated together.',
        inputSchema: editSchema,
      }, async ({path,sha256,oldText,newText,additional}) => result(await track(() => workspace.edit(path,sha256,oldText,newText,additional))));
      mcp.registerTool('project_edit_many',{
        description:'Batch up to 8 files. Each file is verified independently; successful files stay changed if another fails.',
        inputSchema:{edits:z.array(z.object(editSchema)).min(1).max(8)},
      },async({edits})=>result(await track(()=>Promise.all(edits.map(async edit=>{
        try {return await workspace.edit(edit.path,edit.sha256,edit.oldText,edit.newText,edit.additional);}
        catch(error) {return {path:edit.path,applied:false,error:String(error.message)};}
      })))));
      if(options.history)mcp.registerTool('conversation_read',{description:'Read a bounded page of earlier messages in the active conversation when the handoff excerpt is insufficient.',inputSchema:{offset:z.number().int().min(0).default(0),limit:z.number().int().min(1).max(16000).default(8000)}},async({offset,limit})=>result(await track(async()=>{
        const {messages}=await options.history!();const text=messages.filter(m=>m.role==='user'||m.role==='assistant').map(m=>`${m.role}: ${m.text || ''}${m.attachments?.length ? '\nAttachments: '+m.attachments.map(a=>`.refract-attachments/${a.token} (${a.name})`).join(', ') : ''}`).join('\n\n');
        return {offset,nextOffset:offset+limit<text.length ? offset+limit : null,totalCharacters:text.length,content:text.slice(offset,offset+limit)};
      })));
      const runBuild = async (work: () => Promise<unknown>) => {
        if (busyBuild) throw new Error('A build or package request is already running');
        busyBuild = true;
        try { return result(await track(()=>workspace.exclusive(work))); } finally { busyBuild = false; }
      };
      mcp.registerTool('build', {description: 'Compile with the active Refract engine and root. Reuses a validated unchanged successful build unless force=true. Build once after a coherent batch of edits. Compiler output is bounded; set logLimit for more.', inputSchema: {force:z.boolean().default(false),logLimit:z.number().int().min(1000).max(250000).default(12000)}}, ({force,logLimit}) => runBuild(async()=>{
        successfulBuild = false;
        const value=await build(force);
        successfulBuild = !!value && typeof value === 'object' && 'ok' in value && value.ok === true;
        if(value && typeof value==='object' && 'log' in value && typeof value.log==='string')return {...value,log:value.log.slice(-logLimit),logTruncated:value.log.length>logLimit,logCharacters:value.log.length};
        return value;
      }));
      const setEngine = options.setEngine;
      if (setEngine) mcp.registerTool('project_set_engine', {
        description: 'Change the current project’s saved LaTeX engine. Future builds and the editor use this selection. ' + engineSelectionPolicy,
        inputSchema: z.strictObject({engine: z.enum(toolchain.latexEngines)}),
      }, async ({engine}) => result(await track(() => workspace.exclusive(async () => {
        const change = await setEngine(engine);
        if (change.changed) successfulBuild = false;
        return change;
      }))));
      mcp.registerTool('package_lookup', {description: 'Check availability of a TeX package in the configured TeX environment.', inputSchema: {name: z.string().regex(/^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}\.sty$/)}}, ({name}) => runBuild(() => lookup(name)));
      res.once('close', () => { clients.delete(mcp); void mcp.close(); });
      await mcp.connect(transport);
      await transport.handleRequest(req, res, body);
    })();
    active.add(job);
    void job.catch(() => { if (!res.headersSent) res.writeHead(500).end(); else res.end(); }).finally(() => active.delete(job));
  });
  server.requestTimeout = 15000;
  await new Promise<void>((resolve, reject) => {server.once('error', reject); server.listen(0, '127.0.0.1', () => {server.removeListener('error', reject); resolve();});});
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('Could not start assistant tools');
  const engineInstructions = options.setEngine
    ? (options.initialEngine ? `The current project's LaTeX engine at the start of this turn is ${options.initialEngine}. ` : '') +
      'If asked to compile, call the Refract build tool and check its result before replying, even when the current engine and source need no changes. ' +
      'Use that tool for compilation so the result matches the editor. Call project_set_engine only to change the saved engine. ' +
      engineSelectionPolicy
    : undefined;
  return {
    connection: {url: `http://127.0.0.1:${address.port}/mcp`, token},
    hasSuccessfulBuild: () => successfulBuild,
    engineInstructions,
     instructions: 'Prefer refract tools. Edit directly when the request gives the file and exact old text. Otherwise read targeted context (find for known text), then edit with the returned SHA-256. Batch files with *_many and same-file replacements with additional. A successful edit verifies the save. On failure, reread before retrying; check each batch result. Use conversation_read for omitted history. Use build for requested compilation; batch edits first and check warnings. Only claim visual inspection if performed. Preserve document content and shared TinyTeX; report missing dependencies or use a licensed project-local copy.' +
      (engineInstructions ? ' ' + engineInstructions : ''),
    close() { return closeJob ??= (async () => {
      closing = true;
      const closed = new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
      server.closeIdleConnections();
      await Promise.allSettled([...active]);
      await workspace.drain();
      await Promise.allSettled([...clients].map(client => client.close()));
      server.closeAllConnections();
      await closed;
    })(); },
  };
}
export = {startAssistantTools, createWorkspaceTools};
