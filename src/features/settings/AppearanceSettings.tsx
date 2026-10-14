import type {CSSProperties} from 'react';
import type {Appearance} from '../../contracts/appearance';
import type {SettingStore} from '../../settings/store';
import {useSetting} from '../../settings/use-setting';
import {themes,themeById} from '../../design/themes';
import {InlineMessage,AnimatedIcon,Button} from '../../ui';
import s from './appearance.module.css';
export function AppearanceSettings({store}:{store:SettingStore<Appearance>}){
 const state=useSetting(store);
 const selected=themeById((state.error?store.getConfirmed():state.value)?.themeId??'light').id;
 return <section aria-label="界面配色"><p className={s.description}>为整个界面选择配色，语音控件也会一起切换。</p>
  <div className={s.options} role="group" aria-label="配色">{themes.map(theme=><button type="button" key={theme.id} className={s.option} aria-pressed={selected===theme.id} disabled={!state.ready} onClick={()=>store.update({themeId:theme.id})}>
   <span className={s.preview} style={theme.vars as CSSProperties} aria-hidden="true"><span className={s.sidebar}><i/><i/><i/></span><span className={s.page}><span className={s.title}/><span className={s.row}><span className={s.lines}><i/><i/><i/></span><span className={s.card}>128<span>字</span></span></span><span className={s.panel}><i/><i/></span><span className={s.voice}><i/><i/><i/><i/><i/></span></span></span>
   <span className={s.label}>{theme.name}<span className={s.check}>{selected===theme.id&&<AnimatedIcon icon="check" size={16} enabled={false}/>}</span></span>
  </button>)}</div>
  {state.error&&<><InlineMessage status="error">{state.error}</InlineMessage><Button variant="quiet" onClick={()=>void store.load()}>重新读取</Button></>}
 </section>;
}
