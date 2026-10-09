import {invoke,isTauri} from '@tauri-apps/api/core';
import {parseLanguageSettings,type LanguageSettings,type LanguageSettingsPort,type LearningSettingsPatch} from '../contracts/language';
class NativeLanguage implements LanguageSettingsPort {
 async settings(){return parseLanguageSettings(await invoke('language_settings'));}
 async save(settings:LearningSettingsPatch){return parseLanguageSettings(await invoke('language_save',{settings}));}
}
export class BrowserLanguage implements LanguageSettingsPort {
 private value:LanguageSettings={automaticTerms:{enabled:true,revision:1},personalization:{enabled:true,revision:1}};
 async settings(){return structuredClone(this.value);}
 async save(patch:LearningSettingsPatch){
  if(!patch.automaticTerms&&!patch.personalization)throw {code:'invalid'};
  for(const key of ['automaticTerms','personalization'] as const){if(patch[key]&&patch[key].revision!==this.value[key].revision)throw {code:'conflict'};}
  for(const key of ['automaticTerms','personalization'] as const){const next=patch[key];if(next&&next.enabled!==this.value[key].enabled)this.value[key]={enabled:next.enabled,revision:this.value[key].revision+1};}
  return this.settings();
 }
}
export function createLanguageSettings():LanguageSettingsPort{return isTauri()?new NativeLanguage():new BrowserLanguage();}
