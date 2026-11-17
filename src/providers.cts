const { spawn } = require('node:child_process');
const path = require('node:path');
const openCode = require('./opencode.cjs');

function probe(command:string, args:string[]):Promise<boolean> {
  return new Promise(resolve=>{
    let done=false;
    const finish=(result:boolean)=>{if(done)return;done=true;clearTimeout(timer);resolve(result);};
    const child=spawn(command,args,{windowsHide:true,stdio:'ignore'});
    const timer=setTimeout(()=>{child.kill();finish(false);},20000);
    child.once('error',()=>finish(false));child.once('close',code=>finish(code===0));
  });
}
async function discoverProviders():Promise<{codex:boolean;opencode:boolean}> {
  const script=path.join(process.env.APPDATA || '', 'npm','node_modules','@openai','codex','bin','codex.js');
  const codex=probe('node',[script,'--version']);
  let opencode:Promise<boolean>;
  try {opencode=probe(openCode.executable(),['--version']);} catch {opencode=Promise.resolve(false);}
  const [hasCodex,hasOpenCode]=await Promise.all([codex,opencode]);
  return {codex:hasCodex,opencode:hasOpenCode};
}
export = { discoverProviders, probe };
