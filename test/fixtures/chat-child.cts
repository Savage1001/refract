const {ProjectStore} = require('../../src/project-store.cjs');
const {ChatService} = require('../../src/chat.cjs');
const [root,id] = process.argv.slice(2);
const store = new ProjectStore(root);
const service = new ChatService(store, async (_cwd: string,_session: string,_prompt: string,_images: string[],_signal: AbortSignal,_progress: unknown,onSession: (session:string)=>void) => {
 onSession('session-before-exit');
 return new Promise(() => { setInterval(() => {},1000); });
});
const save = service.save.bind(service);
service.save = async (...args: unknown[]) => { await save(...args); if ((args[1] as {sessionId?:string}).sessionId === 'session-before-exit') process.stdout.write('saved\n'); };
service.send(id,'Start operation').catch((error: Error)=>{console.error(error);process.exitCode=1;});
