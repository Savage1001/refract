// Coordinates project snapshots around an assistant operation without holding
// the global write queue while the external process is running.
class AssistantCoordinator {
 active: Map<string,Promise<any>>;
 controllers: Map<string,AbortController>;
 enqueue: any; prepare: any; capture: any;
 constructor(enqueue,prepare,capture) {this.active=new Map();this.controllers=new Map();this.enqueue=enqueue;this.prepare=prepare;this.capture=capture;}
 isBusy(id) {return this.active.has(id);}
 cancel(id) {const controller=this.controllers.get(id);if(!controller)return false;controller.abort();return true;}
 run(id, work, metrics?: InstanceType<typeof import('./assistant-performance.cjs').AssistantPerformance>) {
  if(this.active.has(id)) throw new Error('An assistant operation is already running for this project');
  const controller=new AbortController();this.controllers.set(id,controller);
  const operation=Promise.resolve().then(async()=>{
   const queued=performance.now();
   await this.enqueue(async()=>{
    metrics?.add('queue',performance.now()-queued);
    if(metrics) {await metrics.measure('prepare',()=>this.prepare(id));await metrics.measure('snapshotBefore',()=>this.capture(id));}
    else {await this.prepare(id);await this.capture(id);}
   });
   let result, failure, failed=false;
   try {result=await work(controller.signal);} catch(error) {failure=error;failed=true;}
   try {const queued=performance.now();await this.enqueue(()=>{metrics?.add('queue',performance.now()-queued);return metrics ? metrics.measure('snapshotAfter',()=>this.capture(id)) : this.capture(id);});}
   catch(error) {throw new Error(`Assistant operation finished, but its recovery snapshot failed: ${error.message}`);}
   if(failed) throw failure;
   return result;
  });
  this.active.set(id,operation);
  const clear=()=>{this.active.delete(id);this.controllers.delete(id);};
  operation.then(clear,clear);
  return operation;
 }
 async waitForAll() {await Promise.allSettled([...this.active.values()]);}
}
module.exports={AssistantCoordinator};
