import {invoke,isTauri} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {parseProcessing,type ProcessingPort,type ProcessingSettings,type ProcessingStatus,type LearningSettingsPatch} from '../contracts/processing';
import {modelExecutable,type ModelConfigPort} from '../contracts/models';
class NativeProcessing implements ProcessingPort {
 async onArchived(callback:()=>void){return listen('processing:archived',callback);}
 async status(){return parseProcessing(await invoke('processing_status'));}
 async save(settings:ProcessingSettings){return parseProcessing(await invoke('processing_save',{settings}));}
 async saveLanguage(settings:LearningSettingsPatch){await invoke('language_save',{settings});return this.status();}
}
export class BrowserProcessing implements ProcessingPort {
 private settings:ProcessingSettings={enabled:true,revision:1};
 private language:ProcessingSettings={enabled:true,revision:1};
 private automaticTerms:ProcessingSettings={enabled:true,revision:1};
 constructor(private models:ModelConfigPort){}
 async status():Promise<ProcessingStatus>{
  const overview=await this.models.overview(),id=overview.defaults.find(d=>d.usage==='text')?.configId;
  const model=overview.configs.find(c=>c.id===id&&modelExecutable(c,'text'));
  return {settings:{...this.settings},personalization:{...this.language},automaticTerms:{...this.automaticTerms},available:!!model&&model.credentialStatus==='present',model:model?.name??null,error:model&&model.credentialStatus!=='present'?'credentialMissing':null};
 }
 async save(settings:ProcessingSettings){if(settings.revision!==this.settings.revision)throw {code:'conflict'};this.settings={enabled:settings.enabled,revision:settings.revision+1};return this.status();}
 async saveLanguage(settings:LearningSettingsPatch){
  if(!settings.automaticTerms&&!settings.personalization)throw {code:'invalid'};
  if(settings.automaticTerms&&settings.automaticTerms.revision!==this.automaticTerms.revision||settings.personalization&&settings.personalization.revision!==this.language.revision)throw {code:'conflict'};
  if(settings.automaticTerms&&settings.automaticTerms.enabled!==this.automaticTerms.enabled)this.automaticTerms={enabled:settings.automaticTerms.enabled,revision:this.automaticTerms.revision+1};
  if(settings.personalization&&settings.personalization.enabled!==this.language.enabled)this.language={enabled:settings.personalization.enabled,revision:this.language.revision+1};
  return this.status();
 }
}
export function createProcessing(models:ModelConfigPort):ProcessingPort{return isTauri()?new NativeProcessing():new BrowserProcessing(models);}
