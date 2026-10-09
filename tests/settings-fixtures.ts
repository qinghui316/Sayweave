import {SettingStore} from '../src/settings/store';
import {inputMessage,type ShortcutConfigPort} from '../src/contracts/dictation';
import type {SpeechPort} from '../src/contracts/speech';
import {BrowserLanguage} from '../src/adapters/language-tauri';
export const recordingSettings=(speech:SpeechPort)=>new SettingStore(()=>speech.preferences(),value=>speech.savePreferences(value));
export const shortcutSettings=(port:ShortcutConfigPort)=>new SettingStore(async()=>(await port.status()).saved,async value=>(await port.save(value)).saved,e=>inputMessage(e&&typeof e==='object'&&'code'in e?String(e.code):null));
export const automaticSettings=()=>{const port=new BrowserLanguage();return new SettingStore(async()=>(await port.settings()).automaticTerms,async automaticTerms=>(await port.save({automaticTerms})).automaticTerms);};
