import fs = require('node:fs/promises');
import os = require('node:os');
import path = require('node:path');
import url = require('node:url');
import playwright = require('playwright');

interface ReviewProject { id:string; name:string; rootFile:string; created:string; engine:string; draftMode:boolean }
interface ReviewComment { id:string; file:string; quote:string; text:string; resolved:boolean; created:string; replies:{text:string}[] }
interface ReviewState { projects:ReviewProject[]; files:Record<string,string>; comments:ReviewComment[]; counters:Record<string,number>; delay:number }
interface ReviewWindow extends Window { store:Record<string,unknown>; fixture:ReviewState }

// The real renderer runs against an isolated IPC fixture, with no Electron or native windows.
function installFixture(data:{pdf:string; source:string}):void {
 const w = window as unknown as ReviewWindow;
 const created = '2026-01-01T12:00:00.000Z';
 const names = ['Spectral methods for fluid dynamics','Conference paper - autumn notes','A short introduction to graph theory'];
 const projects = names.map((name,index) => ({id:`project-${index}`,name,rootFile:'main.tex',created,engine:'pdflatex',draftMode:false}));
 const state:ReviewState = {projects,files:{'main.tex':data.source,'sections/abstract.tex':'% Research notes\nA stable spectral approximation.\n'},comments:[
  {id:'comment-a',file:'main.tex',quote:'Introduction',text:'Clarify the assumptions before stating the main result.',resolved:false,created,replies:[]},
  {id:'comment-b',file:'main.tex',quote:'',text:'Check the references in the related work.',resolved:false,created,replies:[]}
 ],counters:{},delay:0};
 w.fixture = state;
 const pause = async (name:string):Promise<void> => { state.counters[name] = (state.counters[name] ?? 0)+1; if(state.delay) await new Promise(resolve=>setTimeout(resolve,state.delay)); };
 const project = (id:string):ReviewProject => { const value=projects.find(p=>p.id===id);if(!value)throw new Error('Missing fixture project');return value; };
 const chat = {sessionId:null,messages:[{role:'user',text:'How should I structure the introduction?'},{role:'assistant',text:'Start with the **research question**, then connect existing work to your contribution.\n\n1. Set the context.\n2. Identify the gap.\n3. State your contribution.\n\nUse `\\section{Introduction}` to begin the section.'}]};
 let preferences = {model:null as string|null,reasoningEffort:null as string|null,favorites:[] as string[],fastMode:false};
 const tree = ():object[] => {
  const make=(prefix:string):object[] => {
   const names = [...new Set(Object.keys(state.files).filter(name=>name.startsWith(prefix)).map(name=>name.slice(prefix.length).split('/')[0]))];
   return names.map(name=>{
    const file=prefix+name, folder=Object.keys(state.files).some(candidate=>candidate.startsWith(file+'/'));
    return folder ? {name,path:file,children:make(file+'/')} : {name,path:file};
   });
  }; return make('');
 };
 let updateTrack: 'stable' | 'beta' = 'stable';
 w.store = {
  updateRestart:async()=>{await pause('updateRestart');},
  updateCheck:async()=>{await pause('updateCheck');return {track:updateTrack,phase:'idle',message:'You are up to date. Checking every 10 minutes.'};},
  updateStatus:async()=>({track:updateTrack,phase:'idle',message:'You are up to date. Checking every 10 minutes.'}),
  updateTrack:async(track:'stable' | 'beta')=>{updateTrack=track;return {track,phase:'idle',message:'You are up to date. Checking every 10 minutes.'};},
  list:async()=>structuredClone(projects), projectPreview:async()=>null,
  create:async(name:string)=>{await pause('create');const value={...projects[0],id:`project-${projects.length}`,name};projects.push(value);return value;},
  rename:async(id:string,name:string)=>{await pause('rename');project(id).name=name;return project(id);},
  tree:async()=>tree(), files:async()=>Object.keys(state.files),
  openFile:async(_id:string,name:string)=>({kind:'text',text:state.files[name] ?? ''}),
  read:async(_id:string,name:string)=>state.files[name] ?? '',
  write:async(_id:string,name:string,text:string)=>{state.files[name]=text;},
  createFile:async(_id:string,name:string)=>{await pause('createFile');state.files[name]='';},
  createFolder:async(_id:string,name:string)=>{await pause('createFolder');state.files[name+'/notes.tex']='';},
  renameFile:async(id:string,oldName:string,newName:string)=>{await pause('renameFile');state.files[newName]=state.files[oldName];delete state.files[oldName];return project(id);},
  moveFolder:async(id:string,oldName:string,newName:string)=>{await pause('moveFolder');for(const name of Object.keys(state.files))if(name.startsWith(oldName+'/')){state.files[newName+name.slice(oldName.length)]=state.files[name];delete state.files[name];}return project(id);},
  versionRetainedStages:async()=>0, versionSnapshot:async()=>({id:'version-1'}),
  versionList:async()=>[{id:'version-1',created,files:['main.tex','sections/abstract.tex']}],
  versionCompare:async()=>[{name:'main.tex',status:'changed',previous:data.source.replace('Introduction','Background and scope'),current:data.source}],
  chatLoad:async(id:string)=>id==='project-1'?structuredClone(chat):{sessionId:null,messages:[]},
  chatList:async()=>[{id:'default',title:'Research conversation'}], onChatProgress:()=>{},
  discoverProviders:async()=>({codex:true,opencode:true}),
  discoverCodexModels:async()=>({ok:true,models:[{id:'model-a',model:'model-a',displayName:'Research model',description:'Sample model for UI verification',supportedReasoningEfforts:[{effort:'low',description:''},{effort:'medium',description:''},{effort:'high',description:''}],defaultReasoningEffort:'medium',isDefault:true}]}),
  discoverOpenCodeModels:async()=>({ok:true,models:[{id:'opencode/research',name:'Research model',variants:['low','high']}]}),
  codexPreferencesLoad:async()=>structuredClone(preferences), codexPreferencesSave:async(value:typeof preferences)=>{preferences=structuredClone(value);},
  openCodePreferencesLoad:async()=>({model:'opencode/research',reasoningEffort:'low',favorites:[]}), openCodePreferencesSave:async()=>{},
  chatSend:async()=>{await pause('chatSend');return structuredClone(chat);},
  compile:async()=>({ok:true,pdf:data.pdf,log:'Output written on main.pdf.'}),
  commentList:async()=>structuredClone(state.comments),
  commentAdd:async(_id:string,file:string,quote:string,text:string)=>{await pause('commentAdd');state.comments.push({id:`comment-${state.comments.length}`,file,quote,text,resolved:false,created,replies:[]});},
  commentResolve:async(_id:string,id:string,resolved:boolean)=>{await pause('commentResolve');const comment=state.comments.find(c=>c.id===id);if(comment)comment.resolved=resolved;},
  commentReply:async(_id:string,id:string,text:string)=>{await pause('commentReply');state.comments.find(c=>c.id===id)?.replies.push({text});},
  commentLocate:async()=>{await pause('commentLocate');return {start:0,end:10};}, setEngine:async(id:string,engine:string)=>{project(id).engine=engine;return project(id);},
  setDraftMode:async(id:string,draft:boolean)=>{project(id).draftMode=draft;return project(id);},
  deletedProjects:async()=>[],trashList:async()=>[], openExternal:async()=>{}, savePdf:async()=>{},
  search:async()=>[], chatCancel:async()=>{}, assistantPerformance:async()=>null
 };
}

let cachedDocument:Promise<{pdf:string;source:string}>|undefined;
async function fixtureDocument():Promise<{pdf:string;source:string}> {
 if (!cachedDocument) cachedDocument = (async()=>{
  const directory=await fs.mkdtemp(path.join(os.tmpdir(),'refract-headless-paper-'));
  const source='\\documentclass[11pt]{article}\n\\usepackage[margin=1in]{geometry}\n\\title{Spectral methods}\n\\author{Research notes}\n\\date{Working draft}\n\\begin{document}\n\\maketitle\n\\section{Introduction}\nA stable spectral approximation for fluid dynamics.\n\\end{document}\n';
  try {
   await fs.writeFile(path.join(directory,'main.tex'),source);
   const {compile}:{compile(directory:string):Promise<{ok:boolean;pdf?:string;log:string}>}=require('../../src/compiler.cjs');
   const result=await compile(directory);if(!result.ok || !result.pdf)throw new Error(result.log);
   return {pdf:result.pdf,source};
  } finally {await fs.rm(directory,{recursive:true,force:true,maxRetries:5,retryDelay:200});}
 })();
 return cachedDocument;
}

async function openHeadlessUi(colorScheme:'light'|'dark'='light', fixtureData?: {pdf:string;source:string}) {
 const browser=await playwright.chromium.launch({channel:'msedge',headless:true,args:['--allow-file-access-from-files']});
 try {
  const context=await browser.newContext({viewport:{width:1440,height:900},colorScheme,locale:'en-US',timezoneId:'UTC'});
  await context.route(/^https?:/,route=>route.abort());
  await context.addInitScript(installFixture,fixtureData ?? await fixtureDocument());
  const page=await context.newPage();const errors:string[]=[];
  page.on('pageerror',error=>errors.push(error.message));
  page.on('dialog',dialog=>{errors.push(dialog.message());void dialog.dismiss();});
  await page.goto(url.pathToFileURL(path.resolve(__dirname,'../../index.html')).href);
  await page.waitForFunction(()=>!document.body.inert).catch(error=>{throw new Error(`${error.message} Renderer errors: ${errors.join('; ')}`);});
  return {browser,page,errors};
 } catch(error) {await browser.close();throw error;}
}
export = {openHeadlessUi};
