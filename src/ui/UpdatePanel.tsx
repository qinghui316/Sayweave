import {useState,useSyncExternalStore} from 'react';
import type {UpdatePort} from '../contracts/update';
import {Button,InlineMessage} from './index';
import {updateLabel} from './update-state';
import {Brand} from './Brand';
import s from './update-panel.module.css';
export function UpdatePanel({port,onInstall,preparing,error,showIdentity=true}:{port:UpdatePort;onInstall:()=>void;preparing:boolean;error:string|null;showIdentity?:boolean}){
 const value=useSyncExternalStore(port.subscribe,port.snapshot);
 const [commandError,setCommandError]=useState<string|null>(null);
 const [requesting,setRequesting]=useState(false);
 const busy=preparing||requesting||['checking','downloading','verifying','preparing','stopping','installing'].includes(value.state);
 const run=async(action:()=>Promise<void>)=>{if(requesting)return;setCommandError(null);setRequesting(true);try{await action();}catch{setCommandError('操作未完成，请重试。');}finally{setRequesting(false);}};
 const message=error??commandError??value.error??value.blockedReason;
 const saving=preparing&&value.state==='readyToInstall';
 const actionLabel=value.state==='installing'?'正在安装':value.state==='stopping'?'正在停止服务':value.state==='preparing'?'正在准备更新':value.state==='verifying'?'正在验证':value.state==='downloading'?'正在下载':value.state==='checking'?'正在检查':value.state==='failed'?'重试':'检查更新';
 return <div className={s.panel}>
  {showIdentity&&<section className={s.identity}><h3><Brand/></h3><p>{value.build.version||'正在读取版本…'}<span>{value.build.channel==='stable'?'稳定版':value.build.channel==='test'?'验收版':'开发版'}</span></p>{value.build.commit&&<details><summary>构建信息</summary><small>{value.build.commit.slice(0,8)} · {value.build.target}</small></details>}</section>}
  {(value.build.updatesEnabled||value.recovery)&&<section className={s.update} aria-label="软件更新"><div className={s.status}><h3>软件更新</h3><p role="status">{saving?'正在保存设置':value.build.updatesEnabled?updateLabel(value.state,value.version,value.progress):'此构建使用开发渠道'}</p></div>
   {value.state==='downloading'&&<div className={s.track} role="progressbar" aria-label="更新下载进度" aria-valuemin={0} aria-valuemax={100} aria-valuenow={value.progress??undefined}><span style={{transform:`scaleX(${Math.min(100,Math.max(0,value.progress??0))/100})`}}/></div>}
   {message&&<InlineMessage status="error">{message}</InlineMessage>}
   <div className={s.actions}>{value.recovery?<Button variant="primary" disabled={requesting} onClick={()=>void run(port.restart)}>重启当前版本</Button>:value.state==='readyToInstall'?<Button variant="primary" disabled={busy} onClick={onInstall}>{saving?'正在保存设置':'重启并更新'}</Button>:<Button disabled={busy||!value.build.updatesEnabled} onClick={()=>void run(()=>port.check(true))}>{actionLabel}</Button>}
    {['checking','downloading','verifying'].includes(value.state)&&<Button variant="quiet" disabled={requesting} onClick={()=>void run(port.cancel)}>取消</Button>}
   </div>
   {value.notes&&<details className={s.notes}><summary>查看版本说明</summary><div>{value.notes}</div></details>}
  </section>}
 </div>;
}
