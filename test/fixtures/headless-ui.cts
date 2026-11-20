import path = require('node:path');
import url = require('node:url');
import playwright = require('playwright');
import type { BrowserContextOptions } from 'playwright';

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
  onShutdownNotice:()=>{}, onImportRoot:()=>{},
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

function fixtureDocument(): {pdf:string;source:string} {
 const source='\\documentclass[11pt]{article}\n\\usepackage[margin=1in]{geometry}\n\\title{Spectral methods}\n\\author{Research notes}\n\\date{Working draft}\n\\begin{document}\n\\maketitle\n\\section{Introduction}\nA stable spectral approximation for fluid dynamics.\n\\end{document}\n';
 // This fixture exercises real PDF rendering independently of the installed compiler.
 const stream = 'BT /F1 24 Tf 72 690 Td (Spectral methods) Tj /F1 12 Tf 0 -28 Td (Research notes) Tj 0 -20 Td (Working draft) Tj /F1 16 Tf 0 -52 Td (1 Introduction) Tj /F1 12 Tf 0 -25 Td (A stable spectral approximation for fluid dynamics.) Tj ET\n';
 const objects = [
  '<< /Type /Catalog /Pages 2 0 R >>',
  '<< /Type /Pages /Count 1 /Kids [3 0 R] >>',
  '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>',
  '<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>',
  `<< /Length ${stream.length} >>\nstream\n${stream}endstream`
 ];
 let pdf = '%PDF-1.4\n';
 const offsets = objects.map((object, index) => {
  const offset = pdf.length;
  pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
  return `${String(offset).padStart(10, '0')} 00000 n \n`;
 });
 const crossReference = pdf.length;
 pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.join('')}trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${crossReference}\n%%EOF\n`;
 return {pdf:Buffer.from(pdf, 'ascii').toString('base64'),source};
}

async function openHeadlessUi(colorScheme:'light'|'dark'='light', fixtureData?: {pdf:string;source:string},
  options?: Pick<BrowserContextOptions, 'reducedMotion' | 'recordVideo' | 'viewport'>) {
 const browser=await playwright.chromium.launch({channel:'msedge',headless:true,args:['--allow-file-access-from-files']});
 try {
  const context=await browser.newContext({viewport:{width:1440,height:900},colorScheme,locale:'en-US',timezoneId:'UTC', ...options});
  await context.route(/^https?:/,route=>route.abort());
  await context.addInitScript(installFixture,fixtureData ?? fixtureDocument());
  const page=await context.newPage();const errors:string[]=[];
  page.on('pageerror',error=>errors.push(error.message));
  page.on('dialog',dialog=>{errors.push(dialog.message());void dialog.dismiss();});
  await page.goto(url.pathToFileURL(path.resolve(__dirname,'../../index.html')).href);
  await page.waitForFunction(()=>!document.body.inert).catch(error=>{throw new Error(`${error.message} Renderer errors: ${errors.join('; ')}`);});
  return {browser,page,errors};
 } catch(error) {await browser.close();throw error;}
}
export = {openHeadlessUi};
