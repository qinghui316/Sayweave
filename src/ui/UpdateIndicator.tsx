import {useSyncExternalStore} from 'react';
import type {UpdatePort} from '../contracts/update';
import s from './ui.module.css';
import {ActionIcon} from './ActionMenu';
export function updateLabel(state:string,version:string|null,progress:number|null){
 return state==='checking'?'正在检查更新':state==='downloading'?`正在下载更新${progress===null?'':` ${progress}%`}`:state==='verifying'?'正在验证更新包':state==='readyToInstall'?`重启并更新到 ${version}`:state==='preparing'?'正在保存并准备更新':state==='stopping'?'正在结束后台任务':state==='installing'?'正在启动安装器':state==='recovery'?'需要重启当前版本':state==='failed'?'更新失败':'检查更新';
}
export function UpdateIndicator({port,onOpen}:{port:UpdatePort;onOpen:()=>void}){
 const value=useSyncExternalStore(port.subscribe,port.snapshot);
 const active=value.state!=='idle',pending=['checking','verifying','preparing','stopping','installing'].includes(value.state);
 const label=updateLabel(value.state,value.version,value.progress);
 return <button type="button" className={`${s.updateIndicator} ${active?s.updateActive:''}`} aria-label={label} title={label} onClick={onOpen}>
  <svg viewBox="0 0 40 40" aria-hidden="true" className={pending?s.updateSpin:undefined}><circle className={s.updateTrack} cx="20" cy="20" r="15"/><circle className={s.updateProgress} cx="20" cy="20" r="15" pathLength="100" strokeDasharray="100" strokeDashoffset={pending?75:100-(value.progress??0)}/></svg>
  {value.state==='readyToInstall'&&<span className={s.updateGlyph}><ActionIcon name="retry" size={14}/></span>}
 </button>;
}
