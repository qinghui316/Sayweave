import {invoke,isTauri} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {parseLanguageSettings,parseProfileOverview,parseProfileDetails,type ProfileOverview,type ProfileDetails,type LanguageSettings,type LanguageSettingsPort,type LearningSettingsPatch} from '../contracts/language';
class NativeLanguage implements LanguageSettingsPort {
 async settings(){return parseLanguageSettings(await invoke('language_settings'));}
 async save(settings:LearningSettingsPatch){return parseLanguageSettings(await invoke('language_save',{settings}));}
 async overview(after:string|null=null){return parseProfileOverview(await invoke('language_overview',{after}));}
 async profile(scope:string):Promise<ProfileDetails>{return parseProfileDetails(await invoke('language_profile',{scope}));}
 async onChanged(listener:()=>void){return listen('language:changed',listener);}
}
export class BrowserLanguage implements LanguageSettingsPort {
 private value:LanguageSettings={automaticTerms:{enabled:true,revision:1},personalization:{enabled:true,revision:1}};
 async settings(){return structuredClone(this.value);}
 async overview():Promise<ProfileOverview>{return {preferenceCount:0,applicationCount:0,observationCount:0,shared:{scope:'shared',name:'通用习惯',version:0,preferenceCount:0,observationCount:0,updatedAt:0},applications:[],next:null};}
 async profile(scope:string):Promise<ProfileDetails>{return {scope,version:0,updatedAt:0,preferences:[]};}
 async save(patch:LearningSettingsPatch){
  if(!patch.automaticTerms&&!patch.personalization)throw {code:'invalid'};
  for(const key of ['automaticTerms','personalization'] as const){if(patch[key]&&patch[key].revision!==this.value[key].revision)throw {code:'conflict'};}
  for(const key of ['automaticTerms','personalization'] as const){const next=patch[key];if(next&&next.enabled!==this.value[key].enabled)this.value[key]={enabled:next.enabled,revision:this.value[key].revision+1};}
  return this.settings();
 }
}
export function createLanguageSettings():LanguageSettingsPort{return isTauri()?new NativeLanguage():new BrowserLanguage();}
