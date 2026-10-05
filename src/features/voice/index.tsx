import type { Phase } from '../../contracts/runtime';
import {useState} from 'react';
import { VoiceControl, CopyFeedbackButton,Button,IconButton,Icon } from '../../ui';
import {deliveryNotice,type Delivery,type DictationPresentation} from '../../contracts/dictation';
import s from './voice.module.css';
export function VoiceSurface({ phase, start, finish, cancel, motionEnabled = true,level,error,retry=false,cancelLabel,mode='speech',text,delivery,copy,restore,presentation,dismiss }: { phase: Phase; start: () => void; finish: () => void; cancel: () => void; motionEnabled?: boolean;level?:number;error?:string;retry?:boolean;cancelLabel?:string;mode?:'speech'|'demo';text?:string;delivery?:Delivery|null;copy?:()=>Promise<void>;restore?:()=>Promise<void>;presentation?:DictationPresentation;dismiss?:()=>void }) {
  const [recoveryState,setRecovery]=useState<{owner:Delivery|null|undefined;status:'idle'|'pending'|'done'|'error'}>({owner:null,status:'idle'});
  const recovery=recoveryState.owner===delivery?recoveryState.status:'idle';
  async function recover(){if(!restore||recovery==='pending')return;setRecovery({owner:delivery,status:'pending'});try{await restore();setRecovery({owner:delivery,status:'done'});}catch{setRecovery({owner:delivery,status:'error'});}}
  const kind=presentation??(delivery?.state==='confirmed'?'completed':undefined);
  const result=kind==='result',notice=kind==='notice',failure=kind==='error';
  const recoveryUI=kind!=='idle'&&delivery?.restoreError&&recovery!=='done';
  const workingLabel=kind==='preparing'?'准备录音':kind==='recognizing'?'识别中':kind==='delivering'?'正在填入':kind==='completed'?'已填入':undefined;
  return <main className={s.surface}>
    {result&&<section className={s.result} aria-label="口述结果"><header><span>口述结果</span>{dismiss&&<IconButton label="关闭" onClick={dismiss}><Icon name="x" size={16}/></IconButton>}</header><div className={s.resultText}>{text}</div><div className={s.actions}>{copy&&<CopyFeedbackButton onCopy={copy}/>}</div></section>}
    {(notice||failure||recoveryUI)&&<section className={s.notice} aria-label="口述提示"><div className={s.noticeHeading}><span role={failure?'alert':'status'}>{failure?(error??'识别未完成，请重试。'):recoveryUI?'剪贴板恢复未完成。':deliveryNotice(delivery?.error??null)}</span>{dismiss&&<IconButton label="关闭" onClick={dismiss}><Icon name="x" size={16}/></IconButton>}</div>{recoveryUI&&restore&&<Button disabled={recovery==='pending'} onClick={()=>void recover()}>{recovery==='pending'?'正在恢复':'重试恢复剪贴板'}</Button>}</section>}
    {!result&&!notice&&!failure&&<><VoiceControl mode={mode} feedbackLabel={workingLabel} cancelLabel={cancelLabel??(retry?'取消本次重识别':undefined)} level={level} motionEnabled={motionEnabled} phase={phase} onStart={start} onFinish={finish} onCancel={cancel}/>{workingLabel&&<p className={s.stage} role="status">{workingLabel}</p>}</>}
  </main>;
}
