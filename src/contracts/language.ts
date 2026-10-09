import {parseSetting,type ProcessingSettings} from './processing';
export interface LanguageSettings {automaticTerms:ProcessingSettings;personalization:ProcessingSettings}
export interface LearningSettingsPatch {automaticTerms?:ProcessingSettings;personalization?:ProcessingSettings}
export interface LanguageSettingsPort {settings():Promise<LanguageSettings>;save(patch:LearningSettingsPatch):Promise<LanguageSettings>}
export function parseLanguageSettings(value:unknown):LanguageSettings {
 if(!value||typeof value!=='object')throw Error('Invalid language settings');
 const v=value as Record<string,unknown>;
 return {automaticTerms:parseSetting(v.automaticTerms),personalization:parseSetting(v.personalization)};
}
