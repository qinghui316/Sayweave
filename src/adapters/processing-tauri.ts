import {invoke,isTauri} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {parseProcessing,parseSetting,type ProcessingPort,type ProcessingSettings,type ProcessingStatus} from '../contracts/processing';
import {modelExecutable,type ModelConfigPort} from '../contracts/models';
class NativeProcessing implements ProcessingPort {
 async onArchived(callback:()=>void){return listen('processing:archived',callback);}
 async settings(){return parseSetting(await invoke('processing_settings'));}
 async status(){return parseProcessing(await invoke('processing_status'));}
 async save(settings:ProcessingSettings){return parseSetting(await invoke('processing_save',{settings}));}
}
export class BrowserProcessing implements ProcessingPort {
 private value:ProcessingSettings={enabled:true,revision:1};
 constructor(private models:ModelConfigPort){}
 async settings(){return {...this.value};}
 async status():Promise<ProcessingStatus>{
  const overview=await this.models.overview(),id=overview.defaults.find(d=>d.usage==='text')?.configId;
  const model=overview.configs.find(c=>c.id===id&&modelExecutable(c,'text'));
  return {settings:{...this.value},available:!!model&&model.credentialStatus==='present',model:model?.name??null,error:model&&model.credentialStatus!=='present'?'credentialMissing':null};
 }
 async save(settings:ProcessingSettings){if(settings.revision!==this.value.revision)throw {code:'conflict'};if(settings.enabled!==this.value.enabled)this.value={enabled:settings.enabled,revision:this.value.revision+1};return this.settings();}
}
export function createProcessing(models:ModelConfigPort):ProcessingPort{return isTauri()?new NativeProcessing():new BrowserProcessing(models);}
