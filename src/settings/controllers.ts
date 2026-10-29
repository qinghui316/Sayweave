import type {TranslationPort} from '../contracts/translation';
import type {ProcessingPort} from '../contracts/processing';
import type {LanguageSettingsPort} from '../contracts/language';
import {speechError,type SpeechPort} from '../contracts/speech';
import {inputMessage,type ShortcutConfigPort} from '../contracts/dictation';
import {speechMessages} from '../locales/speech';
import type {AppearancePort} from '../contracts/appearance';
import {SettingStore} from './store';
export function createSettingsStores(processing:ProcessingPort,language:LanguageSettingsPort,speech:SpeechPort,shortcuts:ShortcutConfigPort,appearance:AppearancePort,translation:TranslationPort){
 return {
  translation:new SettingStore(async()=>(await translation.read()).settings,async value=>(await translation.save(value)).settings),
  appearance:new SettingStore(()=>appearance.read(),value=>appearance.save(value)),
  polish:new SettingStore(()=>processing.settings(),value=>processing.save(value)),
  automaticTerms:new SettingStore(async()=>(await language.settings()).automaticTerms,async automaticTerms=>(await language.save({automaticTerms})).automaticTerms),
  personalization:new SettingStore(async()=>(await language.settings()).personalization,async personalization=>(await language.save({personalization})).personalization),
  recording:new SettingStore(()=>speech.preferences(),value=>speech.savePreferences(value),error=>speechMessages[speechError(error).code]),
  shortcuts:new SettingStore(async()=>(await shortcuts.status()).saved,async value=>(await shortcuts.save(value)).saved,error=>inputMessage(error&&typeof error==='object'&&'code'in error?String(error.code):null)),
 };
}
export type SettingsStores=ReturnType<typeof createSettingsStores>;
