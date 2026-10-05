import {SpeechError} from './speech';
export interface TextServiceStatus {installed:boolean;enabled:boolean;updateAvailable:boolean;payloadAvailable:boolean;cleanupPending:boolean;error:string|null}
export type TextServiceAction='enable'|'disable'|'install'|'remove';
export interface TextServicePort {readonly mode:'native'|'simulation';status():Promise<TextServiceStatus>;act(action:TextServiceAction):Promise<TextServiceStatus>}
export function parseTextService(value:unknown):TextServiceStatus {
 if(!value||typeof value!=='object'||Array.isArray(value))throw new SpeechError('protocol');
 const v=value as Record<string,unknown>;
 if(!['installed','enabled','updateAvailable','payloadAvailable','cleanupPending'].every(k=>typeof v[k]==='boolean')||(v.error!==null&&typeof v.error!=='string')||(v.enabled&&!v.installed)||(v.updateAvailable&&(!v.installed||!v.payloadAvailable)))throw new SpeechError('protocol');
 return v as unknown as TextServiceStatus;
}
export function componentMessage(code:string):string {return ({busy:'请在本次口述结束后操作。',componentRegistration:'输入组件需要检查，可尝试更新组件。',componentMissing:'请先安装输入组件。',componentPayloadMissing:'当前程序未包含组件，请使用桌面开发版。',componentInstallCancelled:'已取消安装。',componentInstallFailed:'组件操作未完成，请重试。',componentInstallPending:'组件操作仍在处理中，请稍后检查状态。',componentEnableFailed:'未能启用输入组件，请重试。'}[code]??'暂时无法读取输入组件，请重试。');}
