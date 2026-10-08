import {useCallback,useEffect,useRef,useState} from 'react';
import {Button,Dialog,Field,InlineMessage,Switch,Select} from '../../ui';
import {speechError,type RecordingPreferences,type SpeechPort} from '../../contracts/speech';
import {speechMessages} from '../../locales/speech';
import s from './history.module.css';
export type DepartureGuard=(action:()=>void)=>void;
export function SaveSettings({speech,onGuardChange}:{speech:SpeechPort;onGuardChange?:(guard:DepartureGuard|null)=>void}){
 const [saved,setSaved]=useState<RecordingPreferences|null>(null),[draft,setDraft]=useState<RecordingPreferences|null>(null),[busy,setBusy]=useState(false),[error,setError]=useState(''),[notice,setNotice]=useState(''),[leaving,setLeaving]=useState(false);
 const alive=useRef(false),working=useRef(false),pending=useRef<(()=>void)|null>(null);
 const dirty=!!saved&&!!draft&&(saved.historyEnabled!==draft.historyEnabled||saved.audioEnabled!==draft.audioEnabled||saved.retentionDays!==draft.retentionDays);
 useEffect(()=>{alive.current=true;let disposed=false;speech.preferences().then(p=>{if(!disposed){setSaved(p);setDraft(p);}}).catch(e=>{if(!disposed)setError(speechMessages[speechError(e).code]);});return()=>{disposed=true;alive.current=false;};},[speech]);
 const guard=useCallback((action:()=>void)=>{if(working.current)return;if(dirty){pending.current=action;setLeaving(true);}else action();},[dirty]);
 useEffect(()=>{onGuardChange?.(guard);return()=>onGuardChange?.(null);},[guard,onGuardChange]);
 async function retryLoad(){if(working.current)return;working.current=true;setBusy(true);setError('');try{const value=await speech.preferences();if(alive.current){setSaved(value);setDraft(value);}}catch(e){if(alive.current)setError(speechMessages[speechError(e).code]);}finally{working.current=false;if(alive.current)setBusy(false);}}
 async function save(){if(!draft||!saved||working.current)return;working.current=true;setBusy(true);setError('');try{
  const current=await speech.preferences();
  if(current.historyEnabled!==saved.historyEnabled||current.audioEnabled!==saved.audioEnabled||current.retentionDays!==saved.retentionDays){setSaved(current);throw speechError({code:'conflict'});}
  const value=await speech.savePreferences({...current,historyEnabled:draft.historyEnabled,audioEnabled:draft.audioEnabled,retentionDays:draft.retentionDays});
  if(alive.current){setSaved(value);setDraft(value);setNotice('已保存，下一次录音生效。');}
 }catch(e){if(alive.current)setError(speechMessages[speechError(e).code]);}finally{working.current=false;if(alive.current)setBusy(false);}}
 return <div className={s.preferences}>
  {draft&&<><div className={s.preferenceRow}><div><strong><label htmlFor="save-history">保存历史</label></strong><p className={s.meta}>识别文字保存在这台设备上。</p></div><Switch id="save-history" disabled={busy} label="保存历史" checked={draft.historyEnabled} onCheckedChange={v=>{setDraft({...draft,historyEnabled:v});setNotice('');}}/></div>
  <div className={s.preferenceRow}><div><strong><label htmlFor="save-audio">保留录音</label></strong><p className={s.meta}>用于回放和重新识别。</p></div><Switch id="save-audio" label="保留录音" disabled={busy||!draft.historyEnabled} checked={draft.audioEnabled} onCheckedChange={value=>{setDraft({...draft,audioEnabled:value});setNotice('');}}/></div>
  <Field label="录音保留时间">{p=><Select {...p} label="录音保留时间" disabled={busy||!draft.historyEnabled||!draft.audioEnabled} value={String(draft.retentionDays??'forever')} onValueChange={value=>{setDraft({...draft,retentionDays:value==='forever'?null:Number(value) as 7|30|90});setNotice('');}} options={[{value:'7',label:'7 天'},{value:'30',label:'30 天'},{value:'90',label:'90 天'},{value:'forever',label:'永久'}]}/>}</Field>
  <p className={s.meta}>录音到期后，文字仍会保留。使用云端识别时，录音会发送给所选服务。</p><div className={s.actions}><Button variant="primary" disabled={!dirty} loading={busy} onClick={()=>void save()}>保存设置</Button>{dirty&&<span className={s.meta}>有未保存的修改</span>}</div></>}
  {error&&<InlineMessage status="error">{error}</InlineMessage>}{!draft&&error&&<Button loading={busy} onClick={()=>void retryLoad()}>重试读取设置</Button>}{notice&&<InlineMessage status="success">{notice}</InlineMessage>}
  <Dialog confirmation open={leaving} onOpenChange={v=>{if(!v){setLeaving(false);pending.current=null;}}} title="有未保存的修改" description="要保留这次填写吗？"><div className={s.actions}><Button variant="primary" onClick={()=>{setLeaving(false);pending.current=null;}}>继续填写</Button><Button onClick={()=>{setDraft(saved);setLeaving(false);const action=pending.current;pending.current=null;action?.();}}>放弃修改</Button></div></Dialog>
 </div>;
}
