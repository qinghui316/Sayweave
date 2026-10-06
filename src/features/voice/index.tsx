import type { Phase } from '../../contracts/runtime';
import { VoiceControl, CopyFeedbackButton,Button,IconButton,Icon } from '../../ui';
import {deliveryNotice,type Delivery,type DictationPresentation} from '../../contracts/dictation';
import s from './voice.module.css';
export function VoiceSurface({ phase, start, finish, cancel, motionEnabled = true,level,error,retry=false,cancelLabel,mode='speech',text,delivery,copy,presentation,dismiss,configure }: { phase: Phase; start: () => void; finish: () => void; cancel: () => void; motionEnabled?: boolean;level?:number;error?:string;retry?:boolean;cancelLabel?:string;mode?:'speech'|'demo';text?:string;delivery?:Delivery|null;copy?:()=>Promise<void>;presentation?:DictationPresentation;dismiss?:()=>void;configure?:()=>void }) {
  const kind=presentation??(delivery?.state==='confirmed'?'completed':undefined);
  const result=kind==='result',notice=kind==='notice',failure=kind==='error';
  const workingLabel=kind==='preparing'?'准备录音':kind==='editing'?'编辑中，口述已暂停':kind==='resuming'?'准备继续口述':kind==='recognizing'?'识别中':kind==='delivering'?'正在填入':kind==='completed'?'口述完成':undefined;
  return <main className={s.surface}>
    {result&&<section className={s.result} aria-label="口述结果"><header><span>口述结果</span>{dismiss&&<IconButton label="关闭" onClick={dismiss}><Icon name="x" size={16}/></IconButton>}</header><div className={s.resultText}>{text}</div><div className={s.actions}>{copy&&<CopyFeedbackButton onCopy={copy}/>}</div></section>}
    {(notice||failure)&&<section className={s.notice} aria-label="口述提示"><span role={failure?'alert':'status'}>{failure?(error??'识别未完成，请重试。'):deliveryNotice(delivery?.error??null)}</span>{failure&&configure&&<Button onClick={configure}>设置</Button>}{dismiss&&<IconButton label="关闭" onClick={dismiss}><Icon name="x" size={16}/></IconButton>}</section>}
    {!result&&!notice&&!failure&&<><VoiceControl compact mode={mode} feedbackLabel={workingLabel} cancelLabel={cancelLabel??(retry?'取消本次重识别':undefined)} level={kind==='editing'||kind==='resuming'?0:level} motionEnabled={motionEnabled} phase={phase} onStart={start} onFinish={finish} onCancel={cancel}/>{workingLabel&&<p className={s.stage} role="status">{workingLabel}</p>}</>}
  </main>;
}
