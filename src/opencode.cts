const { spawn } = require('node:child_process');
const { existsSync } = require('node:fs');
const { join, delimiter } = require('node:path');
const { StringDecoder } = require('node:string_decoder');
const { randomBytes } = require('node:crypto');

function executable(): string {
  if (process.platform !== 'win32') return 'opencode';
  const candidates = [join(process.env.APPDATA || '', 'npm', 'node_modules', '@opencode', 'cli', 'bin', 'opencode.exe'), join(process.env.APPDATA || '', 'npm', 'node_modules', 'opencode-ai', 'bin', 'opencode.exe'), join(process.env.USERPROFILE || '', '.opencode', 'bin', 'opencode.exe'), ...(process.env.PATH || '').split(delimiter).map(dir => join(dir, 'opencode.exe'))];
  const found = candidates.find(file => existsSync(file));
  if (!found) throw new Error('OpenCode was not found. Install and authenticate OpenCode before using it.');
  return found;
}
export interface OpenCodeModel { id: string; name: string; variants: string[] }
type OpenCodeModelsResult = { ok: true; models: OpenCodeModel[] } | { ok: false; error: string };
function parseOpenCodeCatalog(value: unknown): OpenCodeModel[] {
  if (!value || typeof value !== 'object' || !Array.isArray((value as any).data)) throw new Error('Invalid OpenCode model catalog');
  return (value as any).data.filter((m: any) => m && typeof m.providerID === 'string' && typeof m.id === 'string' && /^[^\s/#]+$/.test(m.providerID) && /^[^\s#]+$/.test(m.id) && Array.isArray(m.variants)).map((m: any) => ({
    id: `${m.providerID}/${m.id}`,
    name: typeof m.name === 'string' && m.name.trim() ? m.name.trim() : m.id,
    variants: [...new Set<string>(m.variants.map((v: any) => v?.id).filter((id: unknown): id is string => typeof id === 'string' && /^[^\s#\/\x00-\x1f\x7f]{1,200}$/.test(id)))],
  }));
}
function discoverOpenCodeModels(commandOverride?: string, cwd?: string): Promise<OpenCodeModelsResult> {
  return new Promise(resolve => {
    let command: string;
    try { command = commandOverride ?? executable(); } catch { return resolve({ok:false, error:'OpenCode was not found'}); }
    const password: string = randomBytes(32).toString('hex');
    const child = spawn(command, ['serve', '--hostname', '127.0.0.1', '--port', '0'], {windowsHide:true, cwd, stdio:['ignore','pipe','ignore'], env:{...process.env, OPENCODE_SERVER_USERNAME:'opencode', OPENCODE_SERVER_PASSWORD:password}});
    let output = '', done = false, started = false;
    const finish = (result: OpenCodeModelsResult) => { if (done) return; done = true; clearTimeout(timer); child.kill(); resolve(result); };
    const timer = setTimeout(() => finish({ok:false,error:'OpenCode model discovery timed out'}), 20000);
    child.stdout.on('data', (chunk: Buffer) => {
      output += chunk.toString('utf8');
      if (output.length > 16384) return finish({ok:false,error:'Invalid OpenCode server response'});
      const url = output.match(/server listening on (http:\/\/127\.0\.0\.1:\d+)/)?.[1];
      if (!url || started) return;
      started = true;
      void fetch(`${url}/api/model`, {headers:{Authorization:`Basic ${Buffer.from(`opencode:${password}`).toString('base64')}`}, redirect:'error', signal:AbortSignal.timeout(15000)})
        .then(async response => { if (!response.ok) throw new Error('Catalog request failed'); return parseOpenCodeCatalog(await response.json()); })
        .then(models => finish({ok:true,models}), () => finish({ok:false,error:'Could not load OpenCode model catalog'}));
    });
    child.on('error', () => finish({ok:false,error:'Could not start OpenCode model discovery'}));
    child.on('close', () => finish({ok:false,error:'OpenCode model discovery stopped unexpectedly'}));
  });
}
class OpenCodeRunError extends Error {
  constructor(message: string, public sessionId: string | null) { super(message); }
}
class OpenCodeEvents {
  private decoder = new StringDecoder('utf8'); private pending = ''; private dropping = false;
  sessionId: string | null; text = ''; error = ''; oversized = false;
  constructor(sessionId: string | null, private onProgress?: (s: string) => void, private onSession?: (s: string) => void) { this.sessionId = sessionId; }
  write(buffer: Buffer) { this.consume(this.decoder.write(buffer)); }
  end() { this.consume(this.decoder.end()); if (!this.dropping && this.pending) this.parse(this.pending); }
  private consume(data: string) {
    while (data.length) {
      const i = data.indexOf('\n'), part = i < 0 ? data : data.slice(0, i);
      if (!this.dropping) { if (this.pending.length + part.length > 4 * 1024 * 1024) { this.pending = ''; this.dropping = this.oversized = true; } else this.pending += part; }
      if (i < 0) return;
      if (!this.dropping) this.parse(this.pending); this.pending = ''; this.dropping = false; data = data.slice(i + 1);
    }
  }
  private parse(line: string) {
    try {
      const event = JSON.parse(line);
      if (typeof event.sessionID === 'string' && event.sessionID && this.sessionId !== event.sessionID) { this.sessionId = event.sessionID; try { this.onSession?.(event.sessionID); } catch {} }
      if (event.type === 'text' && typeof event.part?.text === 'string') this.text += event.part.text;
      if (event.type === 'error') this.error = String(event.error?.data?.message || event.error?.message || event.error?._tag || 'OpenCode reported an error');
      if (event.type === 'tool_use') try { this.onProgress?.('Using a tool…'); } catch {}
      if (event.type === 'step_start') try { this.onProgress?.('Thinking…'); } catch {}
    } catch {}
  }
}
function openCodeArguments(sessionId: string | null, images: string[], model?: string, variant?: string, standalone=false): string[] {
  return ['run', '--format', 'json', ...(standalone ? ['--standalone'] : []), ...(model ? ['--model', variant ? model + '#' + variant : model] : []), ...(sessionId ? ['--session', sessionId] : []), ...images.flatMap(file => ['--file', file])];
}
function openCodeEnvironment(tools?: import('./assistant-tools.cjs').ToolConnection, base: NodeJS.ProcessEnv = process.env, v2=false): NodeJS.ProcessEnv {
  if (!tools) return {...base};
  const config = JSON.parse(base.OPENCODE_CONFIG_CONTENT || '{}');
  if(v2) {
    const mcp={...config.mcp},servers={...mcp.servers};
    // Preserve an inherited V1 server map when supplying the V2 shape.
    for(const [name,value] of Object.entries(mcp))if(value && typeof value==='object' && 'type' in value && ['local','remote'].includes(String(value.type))) {servers[name]=value;delete mcp[name];}
    servers.refract={type:'remote',url:tools.url,headers:{Authorization:`Bearer ${tools.token}`},oauth:false,codemode:false};
    return {...base,OPENCODE_CONFIG_CONTENT:JSON.stringify({...config,mcp:{...mcp,servers}})};
  }
  return {...base, OPENCODE_CONFIG_CONTENT: JSON.stringify({...config, mcp: {...config.mcp, refract: {type:'remote',url:tools.url,headers:{Authorization:`Bearer ${tools.token}`},oauth:false}}})};
}
const standaloneSupport=new Map<string,boolean>();
function prepareSession(command:string,cwd:string,sessionId:string,signal?:AbortSignal):Promise<void> {
  return new Promise((resolve,reject)=>{
    if(signal?.aborted)return reject(new OpenCodeRunError('Assistant operation cancelled.',sessionId));
    const child=spawn(command,['api','POST',`/api/session/${encodeURIComponent(sessionId)}/interrupt`,'--standalone'],{cwd,windowsHide:true,shell:false,stdio:'ignore'});let timedOut=false;
    const cancel=()=>child.kill();signal?.addEventListener('abort',cancel,{once:true});if(signal?.aborted)cancel();
    const timer=setTimeout(()=>{timedOut=true;child.kill();},20000);
    const cleanup=()=>{clearTimeout(timer);signal?.removeEventListener('abort',cancel);};
    child.once('error',()=>{cleanup();reject(new OpenCodeRunError('Could not prepare the OpenCode session for resumption. Retry or start a new chat.',sessionId));});
    child.once('close',code=>{cleanup();if(signal?.aborted)return reject(new OpenCodeRunError('Assistant operation cancelled.',sessionId));if(timedOut || code!==0)return reject(new OpenCodeRunError('Could not prepare the OpenCode session for resumption. Retry or start a new chat.',sessionId));resolve();});
  });
}
function supportsStandalone(command:string,signal?:AbortSignal):Promise<boolean> {
  if(standaloneSupport.has(command))return Promise.resolve(standaloneSupport.get(command)!);
  return new Promise((resolve,reject)=>{
    const child=spawn(command,['run','--help'],{windowsHide:true,shell:false,stdio:['ignore','pipe','ignore']});let output='',timedOut=false;
    const cancel=()=>child.kill();signal?.addEventListener('abort',cancel,{once:true});if(signal?.aborted)cancel();
    const timer=setTimeout(()=>{timedOut=true;child.kill();},10000);
    child.stdout.on('data',(value:Buffer)=>{output=(output+value.toString()).slice(-64000);});
    const cleanup=()=>{clearTimeout(timer);signal?.removeEventListener('abort',cancel);};
    child.once('error',error=>{cleanup();reject(error);});
    child.once('close',code=>{cleanup();if(signal?.aborted)return reject(new Error('Assistant operation cancelled.'));if(timedOut || code!==0)return reject(new Error('Could not inspect OpenCode CLI capabilities.'));const supported=output.includes('--standalone');standaloneSupport.set(command,supported);resolve(supported);});
  });
}
async function runOpenCode(cwd: string, sessionId: string | null, prompt: string, images: string[] = [], signal?: AbortSignal, onProgress?: (s: string) => void, onSession?: (s: string) => void, options?: import('./chat.cjs').CodexOptions): Promise<{sessionId: string | null; text: string}> {
  if(signal?.aborted)throw new OpenCodeRunError('Assistant operation cancelled.',sessionId);
  const command=executable(),standalone=!!options?.tools && await supportsStandalone(command,signal);
  // A completed V2 CLI can leave an execution claim that queues the next turn
  // indefinitely. Clear only this conversation before admitting its next turn.
  if(standalone && sessionId)await prepareSession(command,cwd,sessionId,signal);
  return new Promise((resolve, reject) => {
    if (signal?.aborted) return reject(new OpenCodeRunError('Assistant operation cancelled.', sessionId));
    const args = openCodeArguments(sessionId, images, options?.model, options?.reasoningEffort,standalone);
    const child = spawn(command, args, { cwd, windowsHide: true, shell: false, stdio: 'pipe', env:openCodeEnvironment(options?.tools,process.env,standalone) });
    const events = new OpenCodeEvents(sessionId, onProgress, onSession); let stderr = ''; let cancelled = false;
    const cancel = () => { cancelled = true; if (child.pid && process.platform === 'win32') { const killer = spawn('taskkill.exe', ['/pid', String(child.pid), '/t', '/f'], { windowsHide: true, stdio: 'ignore' }); killer.on('error', () => child.kill()); } else child.kill(); };
    signal?.addEventListener('abort', cancel, { once: true }); if (signal?.aborted) cancel();
    child.stdout.on('data', (b: Buffer) => events.write(b)); child.stderr.on('data', (b: Buffer) => { stderr = (stderr + b.toString()).slice(-12000); });
    child.on('error', error => { signal?.removeEventListener('abort', cancel); reject(new OpenCodeRunError(error.message, events.sessionId)); });
    child.on('close', code => { signal?.removeEventListener('abort', cancel); events.end(); if (cancelled) return reject(new OpenCodeRunError('Assistant operation cancelled.', events.sessionId)); if (events.error || code !== 0) return reject(new OpenCodeRunError(events.error || stderr.trim() || `OpenCode exited with code ${code}`, events.sessionId)); if (!events.text) return reject(new OpenCodeRunError(events.oversized ? 'OpenCode returned an oversized event' : 'OpenCode returned no response', events.sessionId)); resolve({ sessionId: events.sessionId, text: events.text }); });
    child.stdin.on('error', () => {}); child.stdin.end(prompt);
  });
}
export = { runOpenCode, OpenCodeEvents, OpenCodeRunError, executable, discoverOpenCodeModels, parseOpenCodeCatalog, openCodeArguments, openCodeEnvironment };
