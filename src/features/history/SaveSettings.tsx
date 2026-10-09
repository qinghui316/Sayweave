import {Button,Field,InlineMessage,Switch,Select} from '../../ui';
import type {RecordingPreferences} from '../../contracts/speech';
import type {SettingStore} from '../../settings/store';
import {useSetting} from '../../settings/use-setting';
import s from './history.module.css';
export function SaveSettings({settings}:{settings:SettingStore<RecordingPreferences>}){
 const state=useSetting(settings),value=state.value;
 return <div className={s.preferences}>
  <div className={s.preferenceRow}><div><strong><label htmlFor="save-history">保存历史</label></strong><p className={s.meta}>识别文字保存在这台设备上。</p></div><Switch id="save-history" disabled={!state.ready} label="保存历史" checked={value?.historyEnabled??false} onCheckedChange={historyEnabled=>settings.update({historyEnabled})}/></div>
  <div className={s.preferenceRow}><div><strong><label htmlFor="save-audio">保留录音</label></strong><p className={s.meta}>用于回放和重新识别。</p></div><Switch id="save-audio" label="保留录音" disabled={!state.ready||!value?.historyEnabled} checked={value?.audioEnabled??false} onCheckedChange={audioEnabled=>settings.update({audioEnabled})}/></div>
  <Field label="录音保留时间">{p=><Select {...p} label="录音保留时间" disabled={!state.ready||!value?.historyEnabled||!value?.audioEnabled} value={String(value?.retentionDays??'forever')} onValueChange={next=>settings.update({retentionDays:next==='forever'?null:Number(next) as 7|30|90})} options={[{value:'7',label:'7 天'},{value:'30',label:'30 天'},{value:'90',label:'90 天'},{value:'forever',label:'永久'}]}/>}</Field>
  <p className={s.meta}>录音到期后，文字仍会保留。更改后下一次录音生效。</p>
  {state.error&&<div><InlineMessage status="error">{state.error}</InlineMessage><Button onClick={()=>void settings.load()}>重新读取</Button></div>}
 </div>;
}
