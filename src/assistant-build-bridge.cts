import http = require('node:http');
import crypto = require('node:crypto');
import path = require('node:path');
const {runCompilerProcess} = require('./compiler.cjs');

async function lookupPackage(name:string, project:string, texBin:string, signal?:AbortSignal) {
  const env={...process.env,PATH:`${texBin}${path.delimiter}${process.env.PATH || ''}`};
  const result=await runCompilerProcess(path.join(texBin,'kpsewhich.exe'),[name],project,env,signal);
  return {name,available:result.code === 0,exitCode:result.code,output:result.output};
}

async function configuredBuild(store: {metadata(id:string):Promise<{engine?:string;rootFile?:string;draftMode?:boolean}>}, id:string, texBin:string, signal?:AbortSignal) {
  const metadata=await store.metadata(id);
  const engine=metadata.engine || 'pdflatex';
  if (!['pdflatex','xelatex','lualatex'].includes(engine)) throw new Error('Unsupported LaTeX engine in project settings');
  return {engine,draftMode:metadata.draftMode === true,rootFile:metadata.rootFile || 'main.tex',executable:path.join(texBin,engine+'.exe'),signal};
}

// One short-lived capability for one active assistant operation. No paths or
// commands are accepted from the client; the caller supplies the operations.
async function startAssistantBuildBridge(build: () => Promise<unknown>, lookup: (name: string) => Promise<unknown>) {
  const token = crypto.randomBytes(32).toString('hex');
  let active: Promise<unknown> | undefined;
  let closing = false;
  const server = http.createServer((req, res) => {
    const respond = (status: number, value: unknown) => {
      res.writeHead(status, {'Content-Type': 'application/json', 'Cache-Control': 'no-store'});
      res.end(JSON.stringify(value));
    };
    if (closing || req.method !== 'POST' || req.headers.authorization !== `Bearer ${token}` || req.headers.origin) return respond(403, {error:'Forbidden'});
    let url: URL;
    try { url = new URL(req.url || '/', 'http://localhost'); } catch { return respond(400, {error:'Invalid request'}); }
    if (url.pathname !== '/build' && url.pathname !== '/package') return respond(404, {error:'Unknown operation'});
    const name = url.searchParams.get('name') || '';
    if (url.pathname === '/package' && !/^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}\.sty$/.test(name)) return respond(400, {error:'Expected a package filename ending in .sty'});
    if (active) return respond(409, {error:'A request is already running'});
    active = Promise.resolve().then(() => url.pathname === '/build' ? build() : lookup(name));
    void active.then(value => respond(200,value), error => respond(500,{error:String(error.message || error)})).finally(() => {active=undefined;});
  });
  await new Promise<void>((resolve,reject) => {server.once('error',reject);server.listen(0,'127.0.0.1',()=>{server.removeListener('error',reject);resolve();});});
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('Could not start build bridge');
  const base = `http://127.0.0.1:${address.port}`;
  const instructions = `Refract build interface (available only for this assistant operation): use these independent PowerShell commands\nInvoke-RestMethod -Method Post -Headers @{ Authorization = 'Bearer ${token}' } -Uri '${base}/build' | ConvertTo-Json -Depth 5\nFor package availability: Invoke-RestMethod -Method Post -Headers @{ Authorization = 'Bearer ${token}' } -Uri '${base}/package?name=PACKAGE.sty' (replace PACKAGE). Each build returns its own ID, exit code, compiler output (last 250000 characters), settings, and PDF path on success. The current TeX log is .refract-build/main.log, replaced on each build. Rebuild after edits and inspect warnings; do not claim visual inspection unless performed. Never install into or modify shared TinyTeX. If a dependency is unavailable, explain the blocker or use a suitable project-local dependency with its license; do not remove document content solely to compile.`;
  return {instructions, async close() {closing=true;server.close();server.closeIdleConnections();if(active) await active.catch(()=>{});}};
}
export = {startAssistantBuildBridge, configuredBuild, lookupPackage};
