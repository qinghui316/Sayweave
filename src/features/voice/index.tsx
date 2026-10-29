import {TranslationTarget} from './TranslationTarget';
import type {TranslationPort} from '../../contracts/translation';
import type {DictationSnapshot} from '../../contracts/dictation';
import type {SpeechMeterSource} from '../../contracts/speech';
import type { Phase } from '../../contracts/runtime';
import { VoiceControl, CopyFeedbackButton,Button,IconButton,Icon } from '../../ui';
import {type Delivery,type DictationPresentation} from '../../contracts/dictation';
import s from './voice.module.css';
export function VoiceSurface({ phase, start, finish, cancel, motionEnabled = true,level,meterSource,error,retry=false,cancelLabel,mode='speech',text,delivery,copy,presentation,dismiss,configure,translation,translationPort }: { phase: Phase; start: () => void; finish: () => void; cancel: () => void; motionEnabled?: boolean;level?:number;meterSource?:SpeechMeterSource;error?:string;retry?:boolean;cancelLabel?:string;mode?:'speech'|'demo';text?:string;delivery?:Delivery|null;copy?:()=>Promise<void>;presentation?:DictationPresentation;dismiss?:()=>void;configure?:()=>void;translation?:DictationSnapshot;translationPort?:TranslationPort }) {
  const kind=presentation??(delivery?.state==='confirmed'?'completed':undefined);
  // Older snapshots may contain a terminal notice; never render a status banner.
  if(kind==='notice'||kind==='idle'||kind==='error'&&!error)return null;
  const result=kind==='result',failure=kind==='error';
  const workingLabel=kind==='preparing'?'准备录音':kind==='editing'?'编辑中，口述已暂停':kind==='resuming'?'准备继续口述':kind==='recognizing'?(translation?'翻译中':'识别中'):kind==='delivering'?'正在填入':kind==='completed'?(translation?'翻译完成':'口述完成'):undefined;
  return <main className={s.surface} data-translation={!!translation}>
    {result&&<section className={s.result} aria-label={translation?(translation.translationComplete?'翻译结果':'识别原文'):'口述结果'}><header><span>{translation?(translation.translationComplete?'翻译结果':'识别原文'):'口述结果'}</span>{dismiss&&<IconButton label="关闭" onClick={dismiss}><Icon name="x" size={16}/></IconButton>}</header>{delivery&&delivery.submission!=='notSubmitted'&&<p className={s.resultHint}>{translation?'请先检查原输入框，避免重复填入。':'以下是本轮完整口述，前面的部分文字可能已填入。'}</p>}{error&&<p role="alert" className={s.resultHint}>{error}</p>}<div className={s.resultText}>{text}</div><div className={s.actions}>{copy&&<CopyFeedbackButton onCopy={copy}/>}</div></section>}
    {failure&&<section className={s.notice} aria-label="口述提示"><span className={s.noticeText} role="alert">{error}</span><div className={s.noticeActions}>{configure&&<Button onClick={configure}>打开设置</Button>}{dismiss&&<Button onClick={dismiss}>关闭</Button>}</div></section>}
    {!result&&!failure&&translation&&translationPort&&translation.targetLanguage&&<TranslationTarget snapshot={translation} port={translationPort}/>}
    {!result&&!failure&&<VoiceControl meterSource={kind==='editing'||kind==='resuming'?undefined:meterSource} compact mode={mode} feedbackLabel={workingLabel} cancelLabel={cancelLabel??(retry?'取消本次重识别':undefined)} level={kind==='editing'||kind==='resuming'?0:level} motionEnabled={motionEnabled} phase={phase} onStart={start} onFinish={finish} onCancel={cancel}/>}
  </main>;
}
