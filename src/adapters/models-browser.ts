import {ModelError,localAssetId,modelExecutable,modelExpectation,modelSameOrigin,validateModelInput,validateModelKey,type ModelConfigPort,type ModelOverview,type ModelSave,type ModelRemove,type ModelDefaultRequest,type ModelDraft,type ModelRecord,type LocalAssetStatus,type LocalModelSave} from '../contracts/models';
export class BrowserModels implements ModelConfigPort {
 readonly mode='simulation' as const;
 private view:ModelOverview={configs:[],localConfigs:[],defaults:[{usage:'speech',configId:null,revision:1},{usage:'text',configId:null,revision:1}],cleanupPending:false};
 private asset:LocalAssetStatus={assetId:localAssetId,state:'missing',received:0,bytes:495831520,error:null};
 readonly local={
  status:async()=>structuredClone(this.asset),
  download:async()=>{this.asset={...this.asset,state:'ready',received:this.asset.bytes};return structuredClone(this.asset);},
  cancel:async()=>{this.asset={...this.asset,state:'missing',received:0};},
  delete:async()=>{if(this.view.localConfigs?.length)throw new ModelError('localConfigurationPresent');this.asset={...this.asset,state:'missing',received:0};},
  save:async(req:LocalModelSave)=>{
   if(req.input.assetId!==localAssetId||req.input.engine!=='transcribe-cpp-0.1.3')throw new ModelError('invalid');
   const old=req.id?this.view.localConfigs?.find(c=>c.id===req.id):undefined;
   if(req.id&&!old)throw new ModelError('notFound');
   if(req.revision!==(old?.revision??null))throw new ModelError('conflict');
   const time=Date.now(),record={...req.input,id:old?.id??crypto.randomUUID(),revision:(old?.revision??0)+1,createdAt:old?.createdAt??time,updatedAt:time};
   this.view.localConfigs=old?this.view.localConfigs!.map(c=>c.id===old.id?record:c):[...(this.view.localConfigs??[]),record];
   return this.overview();
  },
 };
 private requests=new Map<string,()=>void>();
 async status(){return {state:'ready' as const,schemaVersion:5,cleanupPending:false};}
 async overview(){return structuredClone(this.view);}
 private old(d:ModelDraft){
  const old=d.id?this.view.configs.find(c=>c.id===d.id):undefined;
  if(d.id&&!old)throw new ModelError('notFound');
  if(old?.revision!==d.revision && (old||d.revision!==null))throw new ModelError('conflict');
  if(d.key.action==='keep'){
   if(!old)throw new ModelError('credentialMissing');
   if(old.protocol!==d.input.protocol||!modelSameOrigin(old.baseUrl,d.input.baseUrl))throw new ModelError('originChanged');
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
  for(const d of this.view.defaults)if(!d.configId&&modelExecutable(record,d.usage)){d.configId=record.id;d.revision++;}
  return this.overview();
 }
 async remove(req:ModelRemove){
  const c=this.view.configs.find(c=>c.id===req.id)??this.view.localConfigs?.find(c=>c.id===req.id);if(!c)throw new ModelError('notFound');if(c.revision!==req.revision)throw new ModelError('conflict');
  this.confirm(req.id,[],req.expectation);this.apply(req.expectation.replacements);this.view.configs=this.view.configs.filter(c=>c.id!==req.id);this.view.localConfigs=this.view.localConfigs?.filter(c=>c.id!==req.id);return this.overview();
 }
 async setDefault(req:ModelDefaultRequest){
  const d=this.view.defaults.find(d=>d.usage===req.usage),c=this.view.configs.find(c=>c.id===req.configId);
  const local=req.usage==='speech'?this.view.localConfigs?.find(c=>c.id===req.configId):undefined;
  if((!local&&(!c||!modelExecutable(c,req.usage)))||!d)throw new ModelError('invalid');if(d.revision!==req.revision)throw new ModelError('conflict');
  d.configId=req.configId;d.revision++;return this.overview();
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
