import {invoke,isTauri} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {parseProcessing,type ProcessingPort,type ProcessingSettings,type ProcessingStatus} from '../contracts/processing';
import {modelExecutable,type ModelConfigPort} from '../contracts/models';
class NativeProcessing implements ProcessingPort {
 async onArchived(callback:()=>void){return listen('processing:archived',callback);}
 async status(){return parseProcessing(await invoke('processing_status'));}
 async save(settings:ProcessingSettings){return parseProcessing(await invoke('processing_save',{settings}));}
}
export class BrowserProcessing implements ProcessingPort {
 private settings:ProcessingSettings={enabled:true,revision:1};
 constructor(private models:ModelConfigPort){}
 async status():Promise<ProcessingStatus>{
  const overview=await this.models.overview(),id=overview.defaults.find(d=>d.usage==='text')?.configId;
  const model=overview.configs.find(c=>c.id===id&&modelExecutable(c,'text'));
  return {settings:{...this.settings},available:!!model&&model.credentialStatus==='present',model:model?.name??null,error:model&&model.credentialStatus!=='present'?'credentialMissing':null};
 }
 async save(settings:ProcessingSettings){if(settings.revision!==this.settings.revision)throw {code:'conflict'};this.settings={enabled:settings.enabled,revision:settings.revision+1};return this.status();}
}
export function createProcessing(models:ModelConfigPort):ProcessingPort{return isTauri()?new NativeProcessing():new BrowserProcessing(models);}
