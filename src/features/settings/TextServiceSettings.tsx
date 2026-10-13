import {useEffect,useRef,useState} from 'react';
import {Button,InlineMessage} from '../../ui';
import {componentMessage,type TextServicePort,type TextServiceAction,type TextServiceStatus} from '../../contracts/text-service';

export function TextServiceSettings({port}:{port:TextServicePort}){
 const [status,setStatus]=useState<TextServiceStatus|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[restart,setRestart]=useState(false);
 const [source,setSource]=useState<TextServicePort|null>(null);
 const generation=useRef({value:0}),pending=useRef(false);
 useEffect(()=>{
  const epoch=generation.current;const id=++epoch.value;pending.current=false;
  void port.status().then(value=>{if(id===generation.current.value){setSource(port);setStatus(value);setError('');setBusy(false);setRestart(false);}}).catch(()=>{if(id===generation.current.value){setSource(port);setStatus(null);setBusy(false);setRestart(false);setError(componentMessage('componentUnavailable'));}});
  return()=>{epoch.value++;};
 },[port]);
 async function act(action?:TextServiceAction){
  if(source!==port||pending.current)return;
  pending.current=true;const id=++generation.current.value;setBusy(true);setError('');
  try{
   const value=action?await port.act(action):await port.status();
   if(id===generation.current.value){setStatus(value);setRestart(action==='install'&&value.restartRequired===true);}
  }catch(e){if(id===generation.current.value)setError(componentMessage(typeof e==='object'&&e&&'code'in e?String(e.code):'componentUnavailable'));}
  finally{if(id===generation.current.value){pending.current=false;setBusy(false);}}
 }
 if(source!==port)return null;
 const blocked=status?.deploymentState==='pending'||status?.deploymentState==='failed';
 const unavailable=status&&(status.error||!status.installed||!status.enabled||status.updateAvailable||blocked);
 if(!unavailable&&!error&&!restart)return null;
 // An installer-owned operation is already in progress; opening settings must not start another.
 if(status?.deploymentState==='pending'&&!status.error&&!error&&!status.maintenanceAvailable)return null;
 const repair=status?.payloadAvailable&&((!blocked&&(!status.installed||status.updateAvailable))||status.maintenanceAvailable);
 const enable=status?.installed&&!status.enabled&&!status.updateAvailable&&!blocked&&!status.error;
 const recovery:TextServiceAction|undefined=repair?'install':enable?'enable':undefined;
 const message=error||(status?.error?componentMessage(status.error):blocked?componentMessage('componentDeploymentPending'):unavailable?'语音输入暂不可用。':'');
 return <section aria-label="输入恢复">
  {message&&<InlineMessage status="error">{message}</InlineMessage>}
  {restart&&<p role="status">请重新打开正在使用的输入应用一次。</p>}
  {busy?<p role="status">正在恢复输入…</p>:recovery?<Button onClick={()=>void act(recovery)}>恢复输入</Button>:error?<Button variant="quiet" onClick={()=>void act()}>重新检查</Button>:null}
 </section>;
}
