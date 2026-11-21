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
  setEngine?: (engine: LatexEngine) => Promise<{engine: LatexEngine; changed: boolean}>;
}
const MAX_BYTES = 2 * 1024 * 1024;
const fileName = z.string().min(1).max(1024).refine(name => !name.split(/[\\/]/).some(part => part.startsWith('.')), 'Private project files are unavailable');
const hash = (text: string) => crypto.createHash('sha256').update(text).digest('hex');
const result = (value: unknown) => ({content: [{type: 'text' as const, text: JSON.stringify(value)}]});
const engineSelectionPolicy =
  'Choose the fastest LaTeX engine that fully meets the user, template, package, font, language and output requirements. ' +
  'Prefer pdfLaTeX when it meets those requirements and there is no project-specific evidence that another engine is faster. ' +
  'When the document needs XeLaTeX or LuaLaTeX, choose the fastest compatible option based on its requirements and any existing timings. ' +
  'Keep the current engine when it meets the requirements and a switch offers no clear benefit. ' +
  'After switching, build and check for compatibility warnings and required output features. Restore the previous engine if the new one proves unsuitable, and tell the user which engine you selected.';

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
    edit: (name: string, expectedHash: string, oldText: string, newText: string) => serialize(async () => {
      if (!oldText || oldText === newText) throw new Error('Supply a nonempty exact match and a different replacement');
      const before = await read(name);
      if (before.sha256 !== expectedHash) throw new Error('File changed since it was read; read again before editing');
      const at = before.content.indexOf(oldText);
      if (at < 0 || before.content.indexOf(oldText, at + 1) >= 0) throw new Error('Expected exactly one match; read the file and supply more context');
      const content = before.content.slice(0, at) + newText + before.content.slice(at + oldText.length);
      if (Buffer.byteLength(content) > MAX_BYTES) throw new Error('Edited file exceeds 2 MiB');
      if (content.includes('\0') || Buffer.from(content).toString('utf8') !== content) throw new Error('Replacement must be valid Unicode text without NUL bytes');
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
      mcp.registerTool('project_read', {description: 'Read a bounded excerpt of project text (default 16000 characters). SHA-256 covers the whole file for editing. Follow nextOffset for more context.', inputSchema: {path: fileName,...range}}, async ({path,offset,limit}) => result(await track(async()=>excerpt(await workspace.read(path),offset,limit))));
      mcp.registerTool('project_read_many', {description:'Read up to 8 independent files in one call. Each excerpt defaults to 8000 characters; errors are reported per file. Prefer this to separate calls.',inputSchema:{paths:z.array(fileName).min(1).max(8),limit:z.number().int().min(1).max(8000).default(8000)}},async({paths,limit})=>result(await track(async()=>(await workspace.readMany(paths)).map(file=>'error' in file ? file : excerpt(file,0,limit)))));
      mcp.registerTool('project_list', {description: 'List project files directly, without starting a shell.', inputSchema: {}}, async () => result(await track(()=>workspace.list())));
      mcp.registerTool('project_edit', {description: 'Apply one exact replacement and verify the saved file. Read first; use the returned SHA-256. Fails on stale content, zero matches, or multiple matches. Preserves all other bytes.', inputSchema: {path: fileName, sha256: z.string().regex(/^[0-9a-f]{64}$/), oldText: z.string().min(1).max(MAX_BYTES), newText: z.string().max(MAX_BYTES)}}, async ({path, sha256, oldText, newText}) => result(await track(()=>workspace.edit(path, sha256, oldText, newText))));
      const editSchema=z.object({path:fileName,sha256:z.string().regex(/^[0-9a-f]{64}$/),oldText:z.string().min(1).max(MAX_BYTES),newText:z.string().max(MAX_BYTES)});
      mcp.registerTool('project_edit_many',{description:'Apply up to 8 independent exact edits in one call. Each edit is verified separately; successes remain applied if another fails. Inspect every result. Build once after the complete batch.',inputSchema:{edits:z.array(editSchema).min(1).max(8)}},async({edits})=>result(await track(()=>Promise.all(edits.map(async edit=>{try{return await workspace.edit(edit.path,edit.sha256,edit.oldText,edit.newText);}catch(error){return {path:edit.path,applied:false,error:String(error.message)};}})))));
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
        const value=await build(force);
        if(value && typeof value==='object' && 'log' in value && typeof value.log==='string')return {...value,log:value.log.slice(-logLimit),logTruncated:value.log.length>logLimit,logCharacters:value.log.length};
        return value;
      }));
      const setEngine = options.setEngine;
      if (setEngine) mcp.registerTool('project_set_engine', {
        description: 'Change the current project’s saved LaTeX engine. Future builds and the editor use this selection. ' + engineSelectionPolicy,
        inputSchema: z.strictObject({engine: z.enum(toolchain.latexEngines)}),
      }, async ({engine}) => result(await track(() => workspace.exclusive(() => setEngine(engine)))));
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
  return {
    connection: {url: `http://127.0.0.1:${address.port}/mcp`, token},
    instructions: 'Use refract project_list, project_read, and project_edit for direct project operations. Prefer project_read_many and project_edit_many to batch independent files into one tool call. Read only relevant excerpts; follow nextOffset when needed. Each edit needs the full-file SHA-256 from a read. Inspect every batch result: successful edits remain applied if another fails. On a failed edit, read again and correct the match before retrying. Use conversation_read for older conversation context omitted from a handoff excerpt. Use build and package_lookup for the configured TeX environment. Finish a coherent batch of edits before building once; reuse a successful unchanged result. Use force=true if a new compiler run is explicitly needed. Keep shell output targeted and bounded; batch independent commands. Never claim success without checking the result. Inspect build warnings; logTruncated indicates a shortened log and logLimit can request more. Do not claim visual inspection unless performed. Never install into or modify shared TinyTeX. If a dependency is unavailable, explain the blocker or use a suitable project-local dependency with its license; do not remove document content solely to compile.' +
      (options.setEngine ? ' Use project_set_engine to save a different engine for this project. ' + engineSelectionPolicy : ''),
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
