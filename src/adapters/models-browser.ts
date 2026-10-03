import {ModelError,modelExpectation,modelSameOrigin,validateModelInput,validateModelKey,type ModelConfigPort,type ModelOverview,type ModelSave,type ModelRemove,type ModelDefaultRequest,type ModelDraft,type ModelRecord} from '../contracts/models';
export class BrowserModels implements ModelConfigPort {
 readonly mode='simulation' as const;
 private view:ModelOverview={configs:[],defaults:[{usage:'speech',configId:null,revision:1},{usage:'text',configId:null,revision:1}],cleanupPending:false};
 private requests=new Map<string,()=>void>();
 async status(){return {state:'ready' as const,schemaVersion:2,cleanupPending:false};}
 async overview(){return structuredClone(this.view);}
 private old(d:ModelDraft){
  const old=d.id?this.view.configs.find(c=>c.id===d.id):undefined;
  if(d.id&&!old)throw new ModelError('notFound');
  if(old?.revision!==d.revision && (old||d.revision!==null))throw new ModelError('conflict');
  if(d.key.action==='keep'){
   if(!old)throw new ModelError('credentialMissing');
   if(!modelSameOrigin(old.baseUrl,d.input.baseUrl))throw new ModelError('originChanged');
  }else validateModelKey(d.key.value);
  return old;
 }
 private confirm(id:string,usages:ModelRecord['usages'],expected:ModelRemove['expectation']|null){
  if(JSON.stringify(modelExpectation(this.view,id,usages))!==JSON.stringify(expected))throw new ModelError('conflict');
 }
 private apply(replacements:ModelRemove['expectation']['replacements']){
  for(const r of replacements){const d=this.view.defaults.find(d=>d.usage===r.usage)!;d.configId=r.configId;d.revision++;}
 }
 async save(req:ModelSave){
  const input=validateModelInput(req.input),old=this.old(req),time=Date.now();
  if(old){this.confirm(old.id,input.usages,req.expectation);this.apply(req.expectation!.replacements);}
  else if(req.expectation!==null)throw new ModelError('invalid');
  const record:ModelRecord={...input,id:old?.id??crypto.randomUUID(),revision:(old?.revision??0)+1,createdAt:old?.createdAt??time,updatedAt:time,credentialStatus:'present'};
  this.view.configs=old?this.view.configs.map(c=>c.id===old.id?record:c):[...this.view.configs,record];
  this.view.configs.sort((a,b)=>a.createdAt-b.createdAt||a.id.localeCompare(b.id));
  for(const d of this.view.defaults)if(!d.configId&&record.usages.includes(d.usage)){d.configId=record.id;d.revision++;}
  return this.overview();
 }
 async remove(req:ModelRemove){
  const c=this.view.configs.find(c=>c.id===req.id);if(!c)throw new ModelError('notFound');if(c.revision!==req.revision)throw new ModelError('conflict');
  this.confirm(req.id,[],req.expectation);this.apply(req.expectation.replacements);this.view.configs=this.view.configs.filter(c=>c.id!==req.id);return this.overview();
 }
 async setDefault(req:ModelDefaultRequest){
  const d=this.view.defaults.find(d=>d.usage===req.usage),c=this.view.configs.find(c=>c.id===req.configId);
  if(!c?.usages.includes(req.usage)||!d)throw new ModelError('invalid');if(d.revision!==req.revision)throw new ModelError('conflict');
  d.configId=c.id;d.revision++;return this.overview();
 }
 private async demo<T>(draft:ModelDraft,id:string,result:T):Promise<T>{
  this.old(draft);
  if(this.requests.has(id))throw new ModelError('conflict');
  if(this.requests.size>=4)throw new ModelError('busy');
  return new Promise((resolve,reject)=>{
   const timer=setTimeout(()=>{this.requests.delete(id);resolve(result);},250);
   this.requests.set(id,()=>{clearTimeout(timer);this.requests.delete(id);reject(new ModelError('cancelled',undefined,id));});
  });
 }
 async fetchModels(d:ModelDraft,id:string){return this.demo(d,id,['demo-text-model','demo-second-model']);}
 async testTextModel(d:ModelDraft,id:string){if(!d.input.modelId.trim())throw new ModelError('invalid','modelId');return this.demo(d,id,{testedAt:Date.now(),elapsedMs:250});}
 async cancelRequest(id:string){this.requests.get(id)?.();}
}

