import readline = require('node:readline');
let turn=0;
let config:Record<string,unknown>={};
const send=(value:unknown)=>process.stdout.write(JSON.stringify(value)+'\n');
const reply=(id:number,result:unknown)=>send({id,result});
readline.createInterface({input:process.stdin}).on('line',line=>{
  const message=JSON.parse(line),p=message.params || {};
  if(message.method==='initialize')return reply(message.id,{});
  if(message.method==='thread/start'||message.method==='thread/resume'){config=p.config || {};return reply(message.id,{thread:{id:p.threadId || 'fixture-thread'}});}
  if(message.method==='turn/start') {
    const id=String(++turn);reply(message.id,{turn:{id}});send({method:'turn/started',params:{threadId:p.threadId,turn:{id}}});
    if(p.input[0].text==='wait')return;
    if(p.input[0].text==='crash')return process.exit(2);
    send({method:'item/completed',params:{threadId:p.threadId,item:{type:'agentMessage',text:p.input[0].text==='config' ? JSON.stringify(config) : `ready:${process.pid}:${turn}`}}});
    send({method:'turn/completed',params:{threadId:p.threadId,turn:{id,status:'completed'}}});return;
  }
  if(message.method==='turn/interrupt') {reply(message.id,{});send({method:'turn/completed',params:{threadId:p.threadId,turn:{id:String(turn),status:'interrupted'}}});return;}
  if(message.method==='thread/unsubscribe') {reply(message.id,{status:'unsubscribed'});send({method:'thread/closed',params:{threadId:p.threadId}});return;}
  if(message.id)reply(message.id,{});
});
