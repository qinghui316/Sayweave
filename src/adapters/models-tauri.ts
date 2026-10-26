import {invoke,isTauri} from '@tauri-apps/api/core';
import {modelError,parseModelOverview,parseModelStatus,parseModelTest,parseModelIds,parseLocalAsset,type ModelConfigPort,type ModelSave,type ModelRemove,type ModelDefaultRequest,type ModelDraft,type LocalModelSave} from '../contracts/models';
import {BrowserModels} from './models-browser';
class TauriModels implements ModelConfigPort {
 readonly mode='native' as const;
 readonly local={
  status:async(assetId:string)=>parseLocalAsset(await this.call('model_asset_status',{assetId})),
  download:async(assetId:string)=>parseLocalAsset(await this.call('model_asset_download',{assetId})),
  cancel:async(assetId:string)=>{await this.call('model_asset_cancel',{assetId});},
  delete:async(assetId:string)=>{await this.call('model_asset_delete',{assetId});},
  save:async(request:LocalModelSave)=>parseModelOverview(await this.call('model_local_save',{request})),
 };
 private async call(command:string,args:Record<string,unknown>={}){try{return await invoke<unknown>(command,args);}catch(e){throw modelError(e);}}
 async status(retry=false){return parseModelStatus(await this.call('model_status',{retry}));}
 async overview(){return parseModelOverview(await this.call('model_overview'));}
 async save(request:ModelSave){return parseModelOverview(await this.call('model_save',{request}));}
 async remove(request:ModelRemove){return parseModelOverview(await this.call('model_remove',{request}));}
 async setDefault(request:ModelDefaultRequest){return parseModelOverview(await this.call('model_default',{request}));}
 async fetchModels(draft:ModelDraft,requestId:string){return parseModelIds(await this.call('model_fetch',{draft,requestId}));}
 async testTextModel(draft:ModelDraft,requestId:string){return parseModelTest(await this.call('model_test',{draft,requestId}));}
 async cancelRequest(requestId:string){await this.call('model_cancel',{requestId});}
}
export function createModels():ModelConfigPort{return isTauri()?new TauriModels():new BrowserModels();}
export async function openModelHelp(help:import('../contracts/models').ModelHelp):Promise<void>{
 if(isTauri()){
  try{await invoke('model_help',{help});}catch(e){throw modelError(e);}
 }else window.open(help==='doubao-api-key'?'https://console.volcengine.com/speech/app':'https://help.aliyun.com/zh/model-studio/get-api-key','_blank','noopener,noreferrer');
}
