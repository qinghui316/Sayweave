import {invoke,isTauri} from '@tauri-apps/api/core';
import {parseTranslation,type TranslationConfiguration,type TranslationPort,type TranslationSettings} from '../contracts/translation';
class NativeTranslation implements TranslationPort {
 async read(){return parseTranslation(await invoke('translation_settings'));}
 async save(config:TranslationSettings){return parseTranslation(await invoke('translation_save',{config}));}
 async select(taskId:string,code:string){await invoke('translation_select',{taskId,code});}
 async menu(taskId:string,open:boolean){await invoke('translation_menu',{taskId,open});}
}
// Browser preview only. Native catalog and prompt names are owned by the backend.
export class BrowserTranslation implements TranslationPort {
 private value:TranslationConfiguration={settings:{targets:['en-US','ja','ko','fr','zh-Hans'],activeTarget:'en-US',revision:1},languages:[{"code": "en-US", "label": "英语（美国）", "instruction": "American English"}, {"code": "en-GB", "label": "英语（英国）", "instruction": "British English"}, {"code": "zh-Hans", "label": "中文（简体）", "instruction": "Simplified Chinese"}, {"code": "zh-Hant", "label": "中文（繁体）", "instruction": "Traditional Chinese"}, {"code": "ja", "label": "日语", "instruction": "Japanese"}, {"code": "ko", "label": "韩语", "instruction": "Korean"}, {"code": "fr", "label": "法语", "instruction": "French"}, {"code": "de", "label": "德语", "instruction": "German"}, {"code": "es", "label": "西班牙语", "instruction": "Spanish"}, {"code": "pt", "label": "葡萄牙语", "instruction": "Portuguese"}, {"code": "ru", "label": "俄语", "instruction": "Russian"}, {"code": "ar", "label": "阿拉伯语", "instruction": "Arabic"}, {"code": "hi", "label": "印地语", "instruction": "Hindi"}, {"code": "th", "label": "泰语", "instruction": "Thai"}, {"code": "vi", "label": "越南语", "instruction": "Vietnamese"}, {"code": "it", "label": "意大利语", "instruction": "Italian"}, {"code": "nl", "label": "荷兰语", "instruction": "Dutch"}, {"code": "tr", "label": "土耳其语", "instruction": "Turkish"}, {"code": "pl", "label": "波兰语", "instruction": "Polish"}, {"code": "uk", "label": "乌克兰语", "instruction": "Ukrainian"}, {"code": "id", "label": "印度尼西亚语", "instruction": "Indonesian"}, {"code": "ms", "label": "马来语", "instruction": "Malay"}]};
 async read(){return structuredClone(this.value);}
 async save(config:TranslationSettings){if(config.revision!==this.value.settings.revision)throw {code:'conflict'};this.value=parseTranslation({...this.value,settings:{...config,revision:config.revision+1}});return this.read();}
 async select(){throw {code:'unavailable'};}
 async menu(){}
}
export function createTranslation():TranslationPort{return isTauri()?new NativeTranslation():new BrowserTranslation();}
