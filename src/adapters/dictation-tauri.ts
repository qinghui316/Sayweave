import {invoke,isTauri} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {parseSpeech} from '../contracts/speech';
import {parseTextService,type TextServicePort,type TextServiceAction,type TextServiceStatus} from '../contracts/text-service';
import {parseDictation,parseShortcut,shortcutIssue,initialDictation,type DictationPort,type DictationSnapshot,type ShortcutConfigPort,type ShortcutConfig,type ShortcutStatus} from '../contracts/dictation';
class NativeShortcuts implements ShortcutConfigPort {
 readonly mode='native' as const;
 async status(){return parseShortcut(await invoke('shortcut_status'));}
 async save(config:ShortcutConfig){return parseShortcut(await invoke('shortcut_save',{config}));}
 async capture(begin:boolean,token?:string){return parseShortcut(await invoke('shortcut_capture',{begin,token:token??null}));}
 async submitCapture(token:string,binding:string){return parseShortcut(await invoke('shortcut_capture_candidate',{token,binding}));}
 async subscribe(callback:(s:ShortcutStatus)=>void){return listen<unknown>('shortcut:status',e=>callback(parseShortcut(e.payload)));}
}
class NativeDictation implements DictationPort {
 async onSettings(callback:()=>void){return listen('dictation:settings',callback);}
 async settings(taskId:string,presentationId:string){await invoke('dictation_settings',{taskId,presentationId});}
 async snapshot(){return parseDictation(await invoke('dictation_snapshot'));}
 async subscribe(callback:(s:DictationSnapshot)=>void){return listen<unknown>('dictation:snapshot',e=>callback(parseDictation(e.payload)));}
 async start(){return parseSpeech(await invoke('dictation_start'));}
 async control(taskId:string,action:'finish'|'cancel'){await invoke('dictation_control',{taskId,action});}
 async copy(taskId:string,presentationId:string){await invoke('dictation_copy',{taskId,presentationId});}
 async dismiss(taskId:string,presentationId:string){await invoke('dictation_dismiss',{taskId,presentationId});}
}
export class BrowserShortcuts implements ShortcutConfigPort {
 readonly mode='simulation' as const;
 private value:ShortcutStatus={sequence:1,captureToken:null,saved:{binding:'AltRight',translationBinding:'AltRight+ShiftRight',mode:'toggle',revision:1},active:{binding:'AltRight',translationBinding:'AltRight+ShiftRight',mode:'toggle',revision:1},capturing:false,pending:false,error:null,capturedBinding:null};
 private listeners=new Set<(s:ShortcutStatus)=>void>();
 private captureKeys=new Set<string>();
 private finishCapture(binding:string){this.value={...this.value,capturedBinding:binding,capturing:false};window.removeEventListener('keydown',this.key,true);window.removeEventListener('keyup',this.key,true);this.captureKeys.clear();this.emit();}
 private key=(e:KeyboardEvent)=>{
  if(!this.value.capturing)return;e.preventDefault();
  if(e.key==='Escape'&&!e.ctrlKey&&!e.altKey&&!e.shiftKey&&!e.metaKey){void this.capture(false);return;}if(e.repeat)return;
  if(e.type==='keyup'){if(this.value.capturedBinding)this.finishCapture(this.value.capturedBinding);return;}
  this.captureKeys.add(e.code);
  const pair=this.captureKeys.has('AltRight')&&this.captureKeys.has('ShiftRight')?'AltRight+ShiftRight':this.captureKeys.has('AltLeft')&&this.captureKeys.has('ShiftLeft')?'AltLeft+ShiftLeft':null;
  const modifier=/^(Alt|Control|Shift)(Left|Right)$/.test(e.code);
  const mods=modifier?[]:[e.ctrlKey?'Control':'',e.altKey?'Alt':'',e.shiftKey?'Shift':'',e.metaKey?'Super':''].filter(Boolean);
  const binding=pair??[...mods,e.code].join('+');
  if(modifier)this.value={...this.value,capturedBinding:binding};else this.finishCapture(binding);
 };
 private emit(){this.value.sequence++;for(const listener of this.listeners)listener(structuredClone(this.value));}
 async status(){this.value.sequence++;return structuredClone(this.value);}
 async subscribe(listener:(s:ShortcutStatus)=>void){this.listeners.add(listener);return()=>{this.listeners.delete(listener);};}
 async save(config:ShortcutConfig){const issue=shortcutIssue(config);if(issue)throw {code:issue};if(config.revision!==this.value.saved.revision)throw {code:'conflict'};const saved={...config,revision:config.revision+1};this.value={...this.value,saved,active:saved};this.emit();return this.status();}
 async capture(begin:boolean,token?:string){if(!begin&&token&&token!==this.value.captureToken)return this.status();this.value={...this.value,capturing:begin,captureToken:begin?(token??crypto.randomUUID()):this.value.captureToken,capturedBinding:null};this.captureKeys.clear();window.removeEventListener('keydown',this.key,true);window.removeEventListener('keyup',this.key,true);if(begin){window.addEventListener('keydown',this.key,true);window.addEventListener('keyup',this.key,true);}this.emit();return this.status();}
}
class NativeTextService implements TextServicePort {
 readonly mode='native' as const;
 async status(){return parseTextService(await invoke('text_service_status'));}
 async act(action:TextServiceAction){return parseTextService(await invoke('text_service_action',{action}));}
}
export class BrowserTextService implements TextServicePort {
 readonly mode='simulation' as const;
 private value:TextServiceStatus={installed:false,enabled:false,updateAvailable:false,payloadAvailable:true,cleanupPending:false,deploymentState:'required',error:null};
 async status(){return {...this.value};}
 async act(action:TextServiceAction){this.value=action==='remove'?{...this.value,installed:false,enabled:false,deploymentState:'required'}:{...this.value,installed:true,enabled:action!=='disable',updateAvailable:false,deploymentState:'ready'};return this.status();}
}
export function createDictationPorts():{shortcuts:ShortcutConfigPort;dictation:DictationPort|null;textService:TextServicePort}{return isTauri()?{shortcuts:new NativeShortcuts(),dictation:new NativeDictation(),textService:new NativeTextService()}:{shortcuts:new BrowserShortcuts(),dictation:null,textService:new BrowserTextService()};}
export {initialDictation};
