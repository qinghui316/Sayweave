import { Button, Icon, ModeRow, VoiceControl } from '../../ui';
import type { Phase } from '../../contracts/runtime';
import { zh } from '../../locales/zh-CN';
import s from './home.module.css';
export function Home({ onShortcuts, onModels, phase, start, cancel, finish, motionEnabled = true }: { onShortcuts: () => void; onModels: () => void; phase: Phase; start: () => void; cancel: () => void; finish: () => void; motionEnabled?: boolean }) {
  const h = zh.home;
  const sentence = ['processing','completed'].includes(phase) ? h.trialSentence : phase === 'recording' ? h.trialSentence.slice(0,12) : h.trialPlaceholder;
  return <section className={s.home}>
    <header className={s.heading}><h1>{zh.homeTitle}</h1></header>
    <div className={s.top}>
      <section className={s.shortcuts} aria-labelledby="shortcuts-title">
        <div className={s.shortcutHeader}><h2 id="shortcuts-title">{h.shortcuts}</h2><p>{h.globalKeys}</p></div>
        <ModeRow icon="mic" title={h.dictation} description={h.dictationDescription} buttonLabel={h.setKey} onClick={onShortcuts}/>
        <ModeRow icon="languages" title={h.translation} description={h.translationDescription} buttonLabel={h.setKey} onClick={onShortcuts}/>
        <ModeRow icon="square-pen" title={h.edit} description={h.editDescription} buttonLabel={h.setKey} onClick={onShortcuts}/>
      </section>
      <section className={s.usageOffset} aria-labelledby="usage-title"><div className={s.backing}/><div className={s.usage}>
        <div className={s.usageHeading}><h2 id="usage-title">{h.usage}</h2><span className={s.badge}><i/>{h.total}</span></div>
        <div className={s.total}><strong>0</strong><span>{h.characters}</span></div>
        <div className={s.stats}><div><span>{h.duration}</span><strong>0 {h.minutes}</strong></div><div><span>{h.inputs}</span><strong>0 {h.times}</strong></div></div>
        <p className={s.usageHelp}>{h.usageEmpty}</p>
      </div></section>
    </div>
    <section className={s.workspace} aria-labelledby="trial-title">
      <div className={s.prepare}><Icon name="settings" size={20}/><div><h2>{h.prepare}</h2><p>{h.prepareDescription}</p></div><Button onClick={onModels}>{h.configure}</Button></div>
      <div className={s.trial}><div className={s.trialHeading}><h2 id="trial-title">{h.trial}</h2><span className={s.badge}>{h.preset}</span></div>
        <div className={s.practice} aria-live="polite">{sentence}</div>
        <div className={s.trialActions}><p>{h.trialHints[phase]}</p><div className={s.dock}><VoiceControl motionEnabled={motionEnabled} phase={phase} onStart={start} onCancel={cancel} onFinish={finish}/></div></div>
      </div>
    </section>
  </section>;
}
