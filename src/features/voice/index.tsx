import type { Phase } from '../../contracts/runtime';
import {useState} from 'react';
import { VoiceControl, CopyFeedbackButton,Button } from '../../ui';
import {inputMessage,type Delivery} from '../../contracts/dictation';
import s from './voice.module.css';
export function VoiceSurface({ phase, start, finish, cancel, motionEnabled = true,level,error,retry=false,cancelLabel,mode='speech',text,delivery,copy,restore }: { phase: Phase; start: () => void; finish: () => void; cancel: () => void; motionEnabled?: boolean;level?:number;error?:string;retry?:boolean;cancelLabel?:string;mode?:'speech'|'demo';text?:string;delivery?:Delivery|null;copy?:()=>Promise<void>;restore?:()=>Promise<void> }) {
  const [recoveryState,setRecovery]=useState<{owner:Delivery|null|undefined;status:'idle'|'pending'|'done'|'error'}>({owner:null,status:'idle'});
  const recovery=recoveryState.owner===delivery?recoveryState.status:'idle';
  async function recover(){if(!restore||recovery==='pending')return;setRecovery({owner:delivery,status:'pending'});try{await restore();setRecovery({owner:delivery,status:'done'});}catch{setRecovery({owner:delivery,status:'error'});}}
  return <main className={s.surface} title={error}>{text&&<div className={s.caption} role="status"><div>{text}</div>{delivery&&<><p>{delivery.state==='confirmed'?'已填入':delivery.state==='unconfirmed'?'请确认输入结果':inputMessage(delivery.error)}</p>{delivery.restoreError&&recovery!=='done'&&<><p>剪贴板恢复未完成。</p>{restore&&<Button disabled={recovery==='pending'} onClick={()=>void recover()}>{recovery==='pending'?'正在恢复':'重试恢复剪贴板'}</Button>}</>}{delivery.state!=='confirmed'&&copy&&<CopyFeedbackButton onCopy={copy}/>}</>}</div>}<VoiceControl mode={mode} cancelLabel={cancelLabel??(retry?'取消本次重识别':undefined)} level={level} motionEnabled={motionEnabled} phase={phase} onStart={start} onFinish={finish} onCancel={cancel}/></main>;
}
