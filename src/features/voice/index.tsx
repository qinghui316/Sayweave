import type {SpeechMeterSource} from '../../contracts/speech';
import type { Phase } from '../../contracts/runtime';
import { VoiceControl, CopyFeedbackButton,Button,IconButton,Icon } from '../../ui';
import {type Delivery,type DictationPresentation} from '../../contracts/dictation';
import s from './voice.module.css';
export function VoiceSurface({ phase, start, finish, cancel, motionEnabled = true,level,meterSource,error,retry=false,cancelLabel,mode='speech',text,delivery,copy,presentation,dismiss,configure,inputMode='revisable' }: { phase: Phase; start: () => void; finish: () => void; cancel: () => void; motionEnabled?: boolean;level?:number;meterSource?:SpeechMeterSource;error?:string;retry?:boolean;cancelLabel?:string;mode?:'speech'|'demo';text?:string;delivery?:Delivery|null;copy?:()=>Promise<void>;presentation?:DictationPresentation;inputMode?:'revisable'|'caret';dismiss?:()=>void;configure?:()=>void }) {
  const kind=presentation??(delivery?.state==='confirmed'?'completed':undefined);
  // Older snapshots may contain a terminal notice; never render a status banner.
  if(kind==='notice'||kind==='idle'||kind==='error'&&!error)return null;
  const result=kind==='result',failure=kind==='error';
  const workingLabel=kind==='preparing'?'准备录音':kind==='editing'?'编辑中，口述已暂停':kind==='resuming'?'准备继续口述':kind==='recognizing'?'识别中':kind==='delivering'?'正在填入':kind==='completed'?'口述完成':undefined;
  return <main className={s.surface}>
    {inputMode==='caret'&&kind==='recording'&&<p className={s.caretHint} role="status">前文保留，继续在光标处输入</p>}
    {result&&<section className={s.result} aria-label="口述结果"><header><span>口述结果</span>{dismiss&&<IconButton label="关闭" onClick={dismiss}><Icon name="x" size={16}/></IconButton>}</header>{delivery&&delivery.submission!=='notSubmitted'&&<p className={s.resultHint}>以下是本轮完整口述，前面的部分文字可能已填入。</p>}{error&&<p role="alert" className={s.resultHint}>{error}</p>}<div className={s.resultText}>{text}</div><div className={s.actions}>{copy&&<CopyFeedbackButton onCopy={copy}/>}</div></section>}
    {failure&&<section className={s.notice} aria-label="口述提示"><span className={s.noticeText} role="alert">{error}</span><div className={s.noticeActions}>{configure&&<Button onClick={configure}>打开设置</Button>}{dismiss&&<Button onClick={dismiss}>关闭</Button>}</div></section>}
    {!result&&!failure&&<VoiceControl meterSource={kind==='editing'||kind==='resuming'?undefined:meterSource} compact mode={mode} feedbackLabel={workingLabel} cancelLabel={cancelLabel??(retry?'取消本次重识别':undefined)} level={kind==='editing'||kind==='resuming'?0:level} motionEnabled={motionEnabled} phase={phase} onStart={start} onFinish={finish} onCancel={cancel}/>}
  </main>;
}
