import {invoke,isTauri} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {parseAppearance,type Appearance,type AppearancePort} from '../contracts/appearance';
class NativeAppearance implements AppearancePort {
 async read(){return parseAppearance(await invoke('appearance_read'));}
 async save(value:Appearance){return parseAppearance(await invoke('appearance_save',{value}));}
 async subscribe(listener:(value:Appearance)=>void){return listen('appearance:changed',event=>listener(parseAppearance(event.payload)));}
}
class BrowserAppearance implements AppearancePort {
 async read(){try{return parseAppearance(JSON.parse(localStorage.getItem('sayweave.appearance')??'null'));}catch{return {themeId:'light',revision:1};}}
 async save(value:Appearance){const current=await this.read();if(value.revision!==current.revision)throw Error('conflict');const next={themeId:value.themeId,revision:current.revision+1};localStorage.setItem('sayweave.appearance',JSON.stringify(next));return next;}
 async subscribe(listener:(value:Appearance)=>void){const changed=(event:StorageEvent)=>{if(event.key==='sayweave.appearance')void this.read().then(listener);};window.addEventListener('storage',changed);return()=>window.removeEventListener('storage',changed);}
}
export function createAppearance():AppearancePort{return isTauri()?new NativeAppearance():new BrowserAppearance();}
