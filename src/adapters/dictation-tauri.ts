import {invoke,isTauri} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {parseSpeech} from '../contracts/speech';
import {parseTextService,type TextServicePort,type TextServiceAction,type TextServiceStatus} from '../contracts/text-service';
import {parseDictation,parseShortcut,initialDictation,type DictationPort,type DictationSnapshot,type ShortcutConfigPort,type ShortcutConfig,type ShortcutStatus} from '../contracts/dictation';
class NativeShortcuts implements ShortcutConfigPort {
 readonly mode='native' as const;
 async status(){return parseShortcut(await invoke('shortcut_status'));}
 async save(config:ShortcutConfig){return parseShortcut(await invoke('shortcut_save',{config}));}
 async capture(begin:boolean,token?:string){return parseShortcut(await invoke('shortcut_capture',{begin,token:token??null}));}
 async subscribe(callback:(s:ShortcutStatus)=>void){return listen<unknown>('shortcut:status',e=>callback(parseShortcut(e.payload)));}
}
class NativeDictation implements DictationPort {
 async snapshot(){return parseDictation(await invoke('dictation_snapshot'));}
 async subscribe(callback:(s:DictationSnapshot)=>void){return listen<unknown>('dictation:snapshot',e=>callback(parseDictation(e.payload)));}
 async start(){return parseSpeech(await invoke('dictation_start'));}
 async control(taskId:string,action:'finish'|'cancel'){await invoke('dictation_control',{taskId,action});}
 async copy(taskId:string,presentationId:string){await invoke('dictation_copy',{taskId,presentationId});}
 async dismiss(taskId:string,presentationId:string){await invoke('dictation_dismiss',{taskId,presentationId});}
 async restoreClipboard(){await invoke('dictation_restore');}
}
export class BrowserShortcuts implements ShortcutConfigPort {
 readonly mode='simulation' as const;
 private value:ShortcutStatus={sequence:1,captureToken:null,saved:{binding:'AltRight',mode:'toggle',revision:1},active:{binding:'AltRight',mode:'toggle',revision:1},capturing:false,pending:false,error:null,capturedBinding:null};
 private listeners=new Set<(s:ShortcutStatus)=>void>();
 private key=(e:KeyboardEvent)=>{if(!this.value.capturing)return;e.preventDefault();if(e.key==='Escape'){void this.capture(false);return;}if(e.repeat)return;const modifier=/^(Alt|Control|Shift)(Left|Right)$/.test(e.code);const mods=modifier?[]:[e.ctrlKey?'Control':'',e.altKey?'Alt':'',e.shiftKey?'Shift':'',e.metaKey?'Super':''].filter(Boolean);this.value={...this.value,capturedBinding:[...mods,e.code].join('+'),capturing:false};window.removeEventListener('keydown',this.key,true);this.emit();};
 private emit(){this.value.sequence++;for(const listener of this.listeners)listener(structuredClone(this.value));}
 async status(){this.value.sequence++;return structuredClone(this.value);}
 async subscribe(listener:(s:ShortcutStatus)=>void){this.listeners.add(listener);return()=>{this.listeners.delete(listener);};}
 async save(config:ShortcutConfig){if(config.revision!==this.value.saved.revision)throw {code:'conflict'};const saved={...config,revision:config.revision+1};this.value={...this.value,saved,active:saved};this.emit();return this.status();}
 async capture(begin:boolean,token?:string){this.value={...this.value,capturing:begin,captureToken:begin?(token??crypto.randomUUID()):this.value.captureToken,capturedBinding:begin?null:this.value.capturedBinding};window.removeEventListener('keydown',this.key,true);if(begin)window.addEventListener('keydown',this.key,true);this.emit();return this.status();}
}
class NativeTextService implements TextServicePort {
 readonly mode='native' as const;
 async status(){return parseTextService(await invoke('text_service_status'));}
 async act(action:TextServiceAction){return parseTextService(await invoke('text_service_action',{action}));}
}
export class BrowserTextService implements TextServicePort {
 readonly mode='simulation' as const;
 private value:TextServiceStatus={installed:false,enabled:false,updateAvailable:false,payloadAvailable:true,cleanupPending:false,error:null};
 async status(){return {...this.value};}
 async act(action:TextServiceAction){this.value=action==='remove'?{...this.value,installed:false,enabled:false}:{...this.value,installed:true,enabled:action!=='disable',updateAvailable:false};return this.status();}
}
export function createDictationPorts():{shortcuts:ShortcutConfigPort;dictation:DictationPort|null;textService:TextServicePort}{return isTauri()?{shortcuts:new NativeShortcuts(),dictation:new NativeDictation(),textService:new NativeTextService()}:{shortcuts:new BrowserShortcuts(),dictation:null,textService:new BrowserTextService()};}
export {initialDictation};
