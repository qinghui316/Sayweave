import type {Phase} from './runtime';
import {SpeechError,type SpeechSnapshot} from './speech';
export type RecordingMode='toggle'|'hold';
export interface ShortcutConfig {binding:string;mode:RecordingMode;revision:number}
export interface ShortcutStatus {sequence:number;captureToken:string|null;saved:ShortcutConfig;active:ShortcutConfig|null;capturing:boolean;pending:boolean;error:string|null;capturedBinding:string|null}
export interface ShortcutConfigPort {readonly mode:'native'|'simulation';status():Promise<ShortcutStatus>;subscribe(callback:(status:ShortcutStatus)=>void):Promise<()=>void>;save(config:ShortcutConfig):Promise<ShortcutStatus>;capture(begin:boolean,token?:string):Promise<ShortcutStatus>}
export interface Delivery {state:'copy'|'confirmed'|'unconfirmed';error:string|null;restoreError:boolean;elapsedMs:number}
export interface DictationSnapshot {sequence:number;taskId:string|null;source:'trial'|'history'|'external';stage:'idle'|'preparing'|'recognizing'|'delivering'|'result';text:string;delivery:Delivery|null;error:string|null}
export interface DictationPort {snapshot():Promise<DictationSnapshot>;subscribe(callback:(snapshot:DictationSnapshot)=>void):Promise<()=>void>;start():Promise<SpeechSnapshot>;control(taskId:string,action:'finish'|'cancel'):Promise<void>;copy(taskId:string):Promise<void>;restoreClipboard():Promise<void>}
export const initialDictation=():DictationSnapshot=>({sequence:1,taskId:null,source:'trial',stage:'idle',text:'',delivery:null,error:null});
const obj=(v:unknown):Record<string,unknown>=>{if(!v||typeof v!=='object'||Array.isArray(v))throw new SpeechError('protocol');return v as Record<string,unknown>;};
const count=(v:unknown)=>typeof v==='number'&&Number.isSafeInteger(v)&&v>=0;
const nullable=(v:unknown)=>v===null||typeof v==='string';
export function parseShortcut(v:unknown):ShortcutStatus {const s=obj(v);function config(v:unknown){const c=obj(v);if(typeof c.binding!=='string'||c.binding.length>128||!['toggle','hold'].includes(String(c.mode))||!count(c.revision)||c.revision===0)throw new SpeechError('protocol');}config(s.saved);if(s.active!==null)config(s.active);if(!count(s.sequence)||!nullable(s.captureToken)||(typeof s.captureToken==='string'&&!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(s.captureToken))||typeof s.capturing!=='boolean'||typeof s.pending!=='boolean'||!nullable(s.error)||!nullable(s.capturedBinding))throw new SpeechError('protocol');return s as unknown as ShortcutStatus;}
export function parseDictation(v:unknown):DictationSnapshot {const s=obj(v);if(!count(s.sequence)||!nullable(s.taskId)||!['trial','history','external'].includes(String(s.source))||!['idle','preparing','recognizing','delivering','result'].includes(String(s.stage))||typeof s.text!=='string'||s.text.length>1024*1024||!nullable(s.error))throw new SpeechError('protocol');if(s.delivery!==null){const d=obj(s.delivery);if(!['copy','confirmed','unconfirmed'].includes(String(d.state))||!nullable(d.error)||typeof d.restoreError!=='boolean'||!count(d.elapsedMs))throw new SpeechError('protocol');}return s as unknown as DictationSnapshot;}
export function shortcutLabel(binding:string):string{return binding.split('+').map(key=>({AltRight:'右 Alt',AltLeft:'左 Alt',ControlRight:'右 Ctrl',ControlLeft:'左 Ctrl',ShiftRight:'右 Shift',ShiftLeft:'左 Shift',Control:'Ctrl',Super:'Win',Space:'空格',CapsLock:'Caps Lock',PageUp:'Page Up',PageDown:'Page Down'}[key]??key.replace(/^Key|^Digit/,''))).join(' + ');}
export function inputMessage(code:string|null):string {return ({shortcutConflict:'此快捷键已被占用，请换一个。',shortcutUnavailable:'快捷键监听未就绪，请重新保存。',invalidShortcut:'请选择单键或组合键。',conflict:'设置已变化，请重新打开后修改。',busy:'当前任务结束后再操作。',preparing:'正在准备快捷键。',stopping:'应用正在退出。',targetChanged:'输入位置已改变，请复制结果。',keysHeld:'请松开按键后复制结果。',clipboardBusy:'剪贴板正被其他应用使用，请复制结果。',clipboardUnsupported:'原剪贴板内容需要保留，请手动复制结果。',clipboardRestore:'剪贴板恢复未完成，可重试恢复。',targetPassword:'此输入框需要手动输入。',targetPermission:'此应用不允许自动填入，请复制结果。',inputComposition:'输入法还在组词，请复制结果。',saveFailed:'历史未保存，结果可复制。',cancelled:'已停止自动填入，结果可复制。',inputUnconfirmed:'请确认输入结果，避免重复粘贴。',inputSubmitted:'文字已提交，请确认输入结果。'}[code??'']??'结果可复制后粘贴。');}

export function dictationPhase(snapshot:DictationSnapshot,phase:Phase,speech:Pick<SpeechSnapshot,'taskId'|'text'>):Phase {
 if(snapshot.stage==='preparing')return 'connecting';
 if(snapshot.stage==='delivering')return 'processing';
 if(snapshot.source==='external'&&speech.taskId===snapshot.taskId){
  if(snapshot.error==='interrupted')return 'failed';
  if(phase==='completed'&&speech.text.trim()&&snapshot.delivery===null&&!snapshot.error)return 'processing';
 }
 return phase;
}
