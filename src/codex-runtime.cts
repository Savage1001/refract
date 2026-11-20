import cp = require('node:child_process');
import path = require('node:path');
import events = require('node:events');
import type { CodexLaunch } from './codex-launch.cjs';
import type chatTypes = require('./chat.cjs');
interface NotificationParams extends Record<string,unknown> {
  threadId?:string;turn?:{id:string;status?:string;error?:{message:string}};item?:{id:string;type:string;text?:string};
}
interface RpcResult {thread?:{id:string};turn?:{id:string};status?:string;data?:unknown[];[key:string]:unknown}
interface Message { id?: number | string; method?: string; params?: NotificationParams; result?: RpcResult; error?: {message: string} }
interface Pending { resolve(value: RpcResult): void; reject(error: Error): void; timer: NodeJS.Timeout }

// One app-server per workspace keeps CLI initialization warm without sharing a
// working directory, active turn, or cancellation scope between projects.
class CodexRuntime extends events.EventEmitter {
  private child: cp.ChildProcessWithoutNullStreams;
  private readonly pending = new Map<number, Pending>();
  private nextId = 0;
  private initialized: Promise<void>;
  private closed = false;
  private busy = false;
  private idle?: NodeJS.Timeout;
  private stopping?: Promise<boolean>;
  private readonly exited: Promise<void>;
  constructor(readonly cwd: string, command = 'node', args = [path.join(process.env.APPDATA || '', 'npm/node_modules/@openai/codex/bin/codex.js'),'app-server','--config','allow_login_shell=false','--config','thread_unload_delay_secs=0'], env?: NodeJS.ProcessEnv) {
    super();
    this.child = cp.spawn(command,args,{cwd,windowsHide:true,shell:false,stdio:'pipe',env});
    this.exited=new Promise(resolve=>this.child.once('close',()=>resolve()));
    this.child.stdin.on('error',()=>this.fail(new Error('Codex connection closed')));
    this.child.stderr.on('data',()=>{}); // Provider diagnostics can contain prompts or capabilities.
    this.child.on('error',()=>this.fail(new Error('Could not start Codex app server')));
    this.child.on('close',()=>this.fail(new Error('Codex app server stopped')));
    // Bound individual protocol messages; never accumulate arbitrary stdout.
    let pending='', dropping=false;
    this.child.stdout.setEncoding('utf8');
    this.child.stdout.on('data',(chunk:string)=>{
      for (const part of chunk.split(/(?<=\n)/)) {
        if (!dropping) {if(pending.length+part.length>8*1024*1024){pending='';dropping=true;}else pending+=part;}
        if(!part.endsWith('\n'))continue;
        if(dropping){this.fail(new Error('Codex returned an oversized protocol event'));return;}
        let message:Message;try{message=JSON.parse(pending);}catch{this.fail(new Error('Codex returned an invalid protocol event'));return;}
        this.receive(message);pending='';
      }
    });
    this.initialized=this.request('initialize',{clientInfo:{name:env ? 'Refract' : 'refract',title:'Refract',version:'0.2.1'}}).then(()=>{this.send({method:'initialized',params:{}});});
    void this.initialized.catch(error=>this.fail(error));
  }
  get available() {return !this.closed;}
  get running() {return this.busy;}
  private send(value: Message) {if(this.closed)throw new Error('Codex connection is closed');this.child.stdin.write(JSON.stringify(value)+'\n');}
  private receive(message: Message) {
    if(typeof message.id==='number' && !message.method) {
      const request=this.pending.get(message.id);if(!request)return;
      clearTimeout(request.timer);this.pending.delete(message.id);
      message.error ? request.reject(new Error(message.error.message)) : request.resolve(message.result ?? {});return;
    }
    if(message.id!==undefined && message.method) {
      // This is a noninteractive client. Never leave approval/input requests hanging.
      this.child.stdin.write(JSON.stringify({id:message.id,error:{code:-32000,message:'Interactive input is unavailable in Refract'}})+'\n');return;
    }
    this.emit('notification',message);
  }
  private fail(error: Error) {
    if(this.closed)return;this.closed=true;clearTimeout(this.idle);
    for(const request of this.pending.values()){clearTimeout(request.timer);request.reject(error);}this.pending.clear();
    this.emit('stopped',error);
    this.stopping=require('./chat.cjs').cancelProcess(this.child);
  }
  request(method: string, params: unknown = {}, timeoutMs=30000): Promise<RpcResult> {
    if(this.closed)return Promise.reject(new Error('Codex connection is closed'));
    return new Promise((resolve,reject)=>{
      const id=++this.nextId;
      const timer=setTimeout(()=>this.fail(new Error(`Codex ${method} timed out`)),timeoutMs);
      this.pending.set(id,{resolve,reject,timer});
      try {this.send({id,method,params:params as Record<string,unknown>});}catch(error){clearTimeout(timer);this.pending.delete(id);reject(error);}
    });
  }
  async ready() {await this.initialized;}
  async close() {
    if(!this.closed)this.fail(new Error('Codex app server closed'));
    await this.stopping;
    await Promise.race([this.exited,new Promise<void>(resolve=>{const timer=setTimeout(resolve,3000);timer.unref();})]);
  }
  async run(sessionId:string|null,prompt:string,images:string[],signal?:AbortSignal,onProgress?:(status:string)=>void,onSession?:(id:string)=>void,options?:chatTypes.CodexOptions):Promise<{sessionId:string;text:string}> {
    if(this.busy)throw new Error('A Codex turn is already running in this workspace');
    if(signal?.aborted)throw new Error('Assistant operation cancelled.');
    this.busy=true;clearTimeout(this.idle);
    let threadId=sessionId,turnId:string|undefined,text='',cancelTimer:NodeJS.Timeout|undefined,ended=false;
    const progress=(status:string)=>{try{onProgress?.(status);}catch{}};
    let finish!:()=>void, rejectTurn!:(error:Error)=>void;
    const complete=new Promise<void>((resolve,reject)=>{finish=resolve;rejectTurn=reject;});void complete.catch(()=>{});
    const toolStarts = new Map<string,number>();
    const stopped=(error:Error)=>rejectTurn(error);
    const notify=(message:Message)=>{
      const p=message.params;if(!p || p.threadId!==threadId)return;
      if(message.method==='turn/started') {turnId=p.turn?.id;if(signal?.aborted)cancel();progress('Thinking…');}
      if(message.method==='item/started')progress(({commandExecution:'Running a command…',fileChange:'Updating project files…',mcpToolCall:'Using a tool…'} as Record<string,string>)[p.item?.type] || 'Working…');
      if(message.method==='item/started' && ['commandExecution','fileChange','webSearch'].includes(p.item?.type)) {toolStarts.set(p.item.id,performance.now());options?.performance?.count('toolCalls');}
      if(message.method==='item/completed' && toolStarts.has(p.item?.id)) {options?.performance?.add('tool',performance.now()-toolStarts.get(p.item.id)!);toolStarts.delete(p.item.id);}
      if(message.method==='item/completed') {if(p.item?.type==='agentMessage' && typeof p.item.text==='string')text=p.item.text;else progress('Thinking…');}
      if(message.method==='turn/completed') {
        ended=true;
        const turn=p.turn;
        if(turn?.status==='completed')finish();else rejectTurn(new Error(turn?.status==='interrupted' ? 'Assistant operation cancelled.' : turn?.error?.message || 'Codex turn failed'));
      }
    };
    const cancel=()=>{
      if(threadId && turnId)void this.request('turn/interrupt',{threadId,turnId}).catch(()=>this.close());
      cancelTimer ??= setTimeout(()=>{void this.close();},5000);
    };
    this.on('notification',notify);this.on('stopped',stopped);signal?.addEventListener('abort',cancel,{once:true});
    try {
      const start=performance.now();await this.ready();
      if(signal?.aborted)throw new Error('Assistant operation cancelled.');
      const config:Record<string,unknown>={allow_login_shell:false,tool_output_token_limit:4000};
      if(options?.tools)config['mcp_servers.refract']={url:options.tools.url,http_headers:{Authorization:`Bearer ${options.tools.token}`},required:true,default_tools_approval_mode:'approve',tool_timeout_sec:300,tools:{project_read:{output_token_limit:32768},project_read_many:{output_token_limit:32768},conversation_read:{output_token_limit:32768}}};
      if(options?.reasoningEffort)config.model_reasoning_effort=options.reasoningEffort;
      if(options?.fastMode !== undefined) {config.service_tier=options.fastMode ? 'fast' : 'default';config['features.fast_mode']=true;}
      const params={cwd:this.cwd,approvalPolicy:'never',sandbox:'workspace-write',model:options?.model ?? null,config};
      const result=await this.request(sessionId ? 'thread/resume' : 'thread/start',sessionId ? {...params,threadId:sessionId,excludeTurns:true} : params);
      threadId=result.thread?.id;if(!threadId)throw new Error('Codex returned no thread ID');try{onSession?.(threadId);}catch{}
      if(signal?.aborted)throw new Error('Assistant operation cancelled.');
      const started=await this.request('turn/start',{threadId,cwd:this.cwd,input:[{type:'text',text:prompt,text_elements:[]},...images.map(file=>({type:'localImage',path:file}))],...(options?.reasoningEffort ? {effort:options.reasoningEffort} : {})});
      turnId=started.turn?.id;if(signal?.aborted)cancel();
      options?.performance?.add('providerStartup',performance.now()-start);
      await complete;
      if(signal?.aborted)throw new Error('Assistant operation cancelled.');
      if(!text)throw new Error('Codex returned no response');
      return {sessionId:threadId,text};
    } catch(error) {if(!this.available || turnId&&!ended)await this.close();Object.assign(error,{sessionId:threadId});throw error;}
    finally {
      clearTimeout(cancelTimer);signal?.removeEventListener('abort',cancel);this.off('notification',notify);this.off('stopped',stopped);
      if(threadId && this.available) {
        let resolveClosed!:()=>void;
        const closed=new Promise<void>(resolve=>resolveClosed=resolve);
        const unloaded=(message:Message)=>{if(message.method==='thread/closed'&&message.params?.threadId===threadId)resolveClosed();};
        this.on('notification',unloaded);this.once('stopped',resolveClosed);
        let timeout:NodeJS.Timeout|undefined;
        try {
          const reply=await this.request('thread/unsubscribe',{threadId});
          if(reply.status==='notLoaded')resolveClosed();
          const settled=await Promise.race([closed.then(()=>true),new Promise<boolean>(resolve=>{timeout=setTimeout(()=>resolve(false),5000);})]);
          if(!settled)await this.close();
        }catch {await this.close();}
        finally {clearTimeout(timeout);this.off('notification',unloaded);this.off('stopped',resolveClosed);}
      }
      this.busy=false;if(this.available){this.idle=setTimeout(()=>{void this.close();},120000);this.idle.unref();}
    }
  }
}
const runtimes=new Map<string,CodexRuntime>();
const runtimeRevisions=new WeakMap<CodexRuntime,number>();
async function runWarmCodex(cwd:string,sessionId:string|null,prompt:string,images:string[]=[],signal?:AbortSignal,onProgress?:(s:string)=>void,onSession?:(s:string)=>void,options?:chatTypes.CodexOptions) {
  if(signal?.aborted)throw new Error('Assistant operation cancelled.');
  cwd=path.resolve(cwd);
  const launch: CodexLaunch | undefined=options?.launch;
  const key=launch?.accountKey ? cwd+'\0'+launch.accountKey : cwd;
  let runtime=runtimes.get(key);
  if(runtime?.available && launch && launch.revision!==runtimeRevisions.get(runtime)) {
    if(runtime.running)throw new Error('A Codex turn is already running in this workspace');
    await runtime.close();
    runtime=undefined;
  }
  if(!runtime?.available){
    if(runtimes.size>=3){const idle=[...runtimes.values()].find(item=>!item.running);if(idle)void idle.close().catch(()=>{});}
    runtime=new CodexRuntime(cwd,launch?.command,launch?.args,launch?.env);
    runtimes.set(key,runtime);
    if(launch)runtimeRevisions.set(runtime,launch.revision);
    const current=runtime;runtime.once('stopped',()=>{if(runtimes.get(key)===current)runtimes.delete(key);});
  }else options?.performance?.count('providerReuses');
  try{return await runtime.run(sessionId,prompt,images,signal,onProgress,onSession,options);}
  finally {const idle=[...runtimes.values()].filter(item=>!item.running);await Promise.all(idle.slice(0,Math.max(0,idle.length-3)).map(item=>item.close()));}
}
async function closeCodexWorkspace(cwd:string) {
  cwd=path.resolve(cwd);
  await Promise.all([...runtimes].filter(([key])=>key===cwd || key.startsWith(cwd+'\0')).map(([,runtime])=>runtime.close()));
}
async function closeIdleAccountRuntimes() {
  await Promise.all([...runtimes].filter(([key,runtime])=>key.includes('\0') && !runtime.running).map(([,runtime])=>runtime.close()));
}
async function closeCodexRuntimes() {await Promise.all([...runtimes.values()].map(runtime=>runtime.close()));runtimes.clear();}
export = {CodexRuntime,runWarmCodex,closeCodexRuntimes,closeCodexWorkspace,closeIdleAccountRuntimes};
