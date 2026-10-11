import {SpeechError} from './speech';
export type ComponentDeploymentState='required'|'pending'|'ready'|'failed';
export interface TextServiceStatus {installed:boolean;enabled:boolean;updateAvailable:boolean;payloadAvailable:boolean;cleanupPending:boolean;deploymentState:ComponentDeploymentState;error:string|null}
export type TextServiceAction='enable'|'disable'|'install'|'remove';
export interface TextServicePort {readonly mode:'native'|'simulation';status():Promise<TextServiceStatus>;act(action:TextServiceAction):Promise<TextServiceStatus>}
export function parseTextService(value:unknown):TextServiceStatus {
 if(!value||typeof value!=='object'||Array.isArray(value))throw new SpeechError('protocol');
 const v=value as Record<string,unknown>;
 if(!['installed','enabled','updateAvailable','payloadAvailable','cleanupPending'].every(k=>typeof v[k]==='boolean')||!['required','pending','ready','failed'].includes(String(v.deploymentState))||(v.error!==null&&typeof v.error!=='string')||(v.enabled&&!v.installed)||(v.updateAvailable&&(!v.installed||!v.payloadAvailable)))throw new SpeechError('protocol');
 return v as unknown as TextServiceStatus;
}
export function componentMessage(code:string):string {return ({compositionDisconnected:'输入组件连接中断，请重新开始口述。',compositionRejected:'未能建立当前输入框的实时草稿，请重新聚焦后重试。',compositionTimeout:'等待输入框就绪超时，请重试。',compositionBusy:'输入组件正在处理另一项操作，请稍后重试。',inputComposition:'请先完成当前输入法组词。',compositionActivationFailed:'输入服务未能激活，请在设置中检查组件状态。',componentNotEnabled:'请在设置中启用实时输入组件。',componentQueryFailed:'无法查询当前用户的输入组件状态，请在设置中检查。',componentVersionMismatch:'输入组件与程序版本不匹配，请重新运行安装包修复。',compositionRuntimeUnavailable:'无法核实目标应用中的输入组件，请重启该应用后重试。',compositionTargetVersion:'目标应用仍在使用旧输入组件，请重启该应用后重试。',compositionUnavailable:'输入目标尚未就绪，请重启目标应用后重试。',targetReadonly:'此输入框不能编辑。',targetPassword:'此输入框需要手动输入。',targetSafetyUnknown:'无法确认输入框的安全属性。',targetChanged:'输入位置已改变，请重新开始。',busy:'请在本次口述结束后操作。',componentRegistration:'输入组件登记需要检查，请在设置中查看状态。',componentMissing:'输入组件尚未部署，请重新运行安装包。',componentPayloadMissing:'当前程序缺少输入组件，请使用完整安装包。',componentInstallCancelled:'已取消安装。',componentInstallFailed:'组件操作未完成，请重试。',componentInstallPending:'组件操作仍在处理中，请稍后检查状态。',componentDeploymentPending:'输入组件维护未完成，请重新运行安装包恢复。',componentEnableFailed:'未能启用输入组件，请重试。'}[code]??'暂时无法读取输入组件，请重试。');}
