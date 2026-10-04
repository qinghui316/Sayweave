import { Button, Icon, IconButton, ModeRow, VoiceControl,CopyFeedbackButton,InlineMessage } from '../../ui';
import type {SpeechSnapshot} from '../../contracts/speech';
import type {Usage} from '../../contracts/history';
import {speechMessages} from '../../locales/speech';
import type { Phase } from '../../contracts/runtime';
import { zh } from '../../locales/zh-CN';
import s from './home.module.css';
export function Home({ onShortcuts, onModels, phase, start, cancel, finish, motionEnabled = true,speech,level,usage,copy,simulation=false,retryCurrent,readyModel,readyDevice,shortcut }: { onShortcuts: () => void; onModels: () => void; phase: Phase; start: () => void; cancel: () => void; finish: () => void; motionEnabled?: boolean;speech?:SpeechSnapshot;level?:number;usage?:Usage|null;copy?:(text:string)=>Promise<void>;simulation?:boolean;retryCurrent?:()=>void;readyModel?:string|null;readyDevice?:string|null;shortcut?:string }) {
  const h = zh.home;
  const active=['connecting','recording','processing'].includes(phase);
  const sentence = speech?.text||'点击麦克风开始，识别文字会出现在这里。';
  return <section className={s.home}>
    <header className={s.heading}><h1>{zh.homeTitle}</h1></header>
    <div className={s.top}>
      <section className={s.shortcuts} aria-labelledby="shortcuts-title">
        <div className={s.shortcutHeader}><h2 id="shortcuts-title">{h.shortcuts}</h2><p>{h.globalKeys}</p></div>
        <ModeRow icon="mic" title={h.dictation} description={h.dictationDescription} buttonLabel={shortcut||h.setKey} onClick={onShortcuts}/>
        <ModeRow icon="languages" title={h.translation} description={h.translationDescription} buttonLabel={h.setKey} onClick={onShortcuts}/>
        <ModeRow icon="square-pen" title={h.edit} description={h.editDescription} buttonLabel={h.setKey} onClick={onShortcuts}/>
      </section>
      <section className={s.usageOffset} aria-labelledby="usage-title"><div className={s.backing}/><div className={s.usage}>
        <div className={s.usageHeading}><h2 id="usage-title">{h.usage}</h2><span className={s.badge}><i/>{h.total}</span></div>
        <div className={s.total}><strong>{usage?.characters??'—'}</strong><span>{h.characters}</span></div>
        <div className={s.stats}><div><span>{h.duration}</span><strong>{usage?Math.round(usage.durationMs/6000)/10:'—'} {h.minutes}</strong></div><div><span>{h.inputs}</span><strong>{usage?.count??'—'} {h.times}</strong></div></div>
        <span className={s.usageHelp}><IconButton label="统计本地保留的成功记录；重识别不重复计次，删除会减少统计。"><Icon name="info" size={16}/></IconButton></span>
      </div></section>
    </div>
    <section className={s.workspace} aria-labelledby="trial-title">
      <div className={s.prepare}><Icon name="settings" size={20}/><div><h2>{active?speech?.modelName||readyModel||'选择语音模型':readyModel||'选择语音模型'}</h2><p>{active?speech?.deviceName||readyDevice||'系统默认麦克风':readyDevice||'系统默认麦克风'}</p></div><Button onClick={onModels}>{h.configure}</Button></div>
      <div className={s.trial}><div className={s.trialHeading}><h2 id="trial-title">开始口述</h2>{simulation&&<span className={s.badge}>浏览器模拟</span>}</div>
        {simulation&&<p>模拟仅验证界面，不采集音频或调用服务。</p>}
        <div className={s.practice} role="region" aria-label="识别文字" tabIndex={0}>{sentence}</div>
        {speech?.error&&<InlineMessage status="error">{speechMessages[speech.error]}</InlineMessage>}
        {speech?.saveError&&<InlineMessage status="error">{phase==='cancelled'?'已取消，但录音清理未完成；请在历史中重试删除。':'识别结果仍可复制，但存档未完整保存。'}</InlineMessage>}
        <div className={s.trialActions}><div><p role="status">{phase==='recording'?`录音中 · ${Math.floor((speech?.durationMs??0)/1000)} 秒`:phase==='processing'?'正在等待最终结果':speech?.saved?'已保存到历史':speech?.error||speech?.saveError?'':(speech?zh.voice.liveHints[phase]:h.trialHints[phase])}</p>{copy&&<CopyFeedbackButton disabled={!speech?.text} onCopy={()=>copy(speech?.text??'')}/>}</div><div className={s.dock}><VoiceControl mode={speech?'speech':'demo'} cancelLabel={speech?.retry?'取消本次重识别':undefined} level={level} motionEnabled={motionEnabled} phase={phase} onStart={start} onCancel={cancel} onFinish={finish}/></div></div>
        {speech?.canRetry&&!['connecting','recording','processing'].includes(phase)&&retryCurrent&&<Button variant="quiet" onClick={retryCurrent}>重新识别</Button>}
        {['connecting','recording','processing'].includes(phase)&&<Button variant="quiet" onClick={cancel}>{speech?.retry?'取消本次重识别':'取消并丢弃'}</Button>}
      </div>
    </section>
  </section>;
}
