import {invoke,isTauri} from '@tauri-apps/api/core';
import {modelError,parseModelOverview,parseModelStatus,parseModelTest,parseModelIds,type ModelConfigPort,type ModelSave,type ModelRemove,type ModelDefaultRequest,type ModelDraft} from '../contracts/models';
import {BrowserModels} from './models-browser';
class TauriModels implements ModelConfigPort {
 readonly mode='native' as const;
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
