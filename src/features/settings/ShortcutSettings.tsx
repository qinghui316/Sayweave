import {useEffect,useRef,useState} from 'react';
import {Button,InlineMessage,Keycap,SingleChoice,SettingsSection,SettingsRow} from '../../ui';
import {inputMessage,shortcutLabel,type ShortcutConfigPort,type ShortcutConfig,type ShortcutStatus} from '../../contracts/dictation';
import type {SettingStore} from '../../settings/store';
import {useSetting} from '../../settings/use-setting';
import s from './settings.module.css';
export function ShortcutSettings({port,settings}:{port:ShortcutConfigPort;settings:SettingStore<ShortcutConfig>}){
 const state=useSetting(settings),draft=state.value;
 const [status,setStatus]=useState<ShortcutStatus|null>(null),[starting,setStarting]=useState(false),[error,setError]=useState('');
 const captureToken=useRef<string|null>(null),mounted=useRef(false),recorder=useRef<HTMLDivElement>(null);
 const acceptStatus=useRef<(value:ShortcutStatus)=>void>(()=>{});
 useEffect(()=>{
  mounted.current=true;let alive=true,sequence=-1,release:(()=>void)|undefined;
  const accept=(value:ShortcutStatus)=>{
   if(!alive||value.sequence<=sequence)return;sequence=value.sequence;setStatus(value);settings.accept(value.saved);
   if(captureToken.current&&value.captureToken===captureToken.current&&!value.capturing){captureToken.current=null;if(value.capturedBinding)settings.update({binding:value.capturedBinding});}
  };
  acceptStatus.current=accept;
  void port.subscribe(accept).then(stop=>{if(alive)release=stop;else stop();}).catch(()=>setError('快捷键暂不可用，请重试。'));
  void port.status().then(accept).catch(()=>{if(alive)setError('快捷键暂不可用，请重试。');});
  return()=>{alive=false;mounted.current=false;const token=captureToken.current;captureToken.current=null;release?.();if(token)void port.capture(false,token).catch(()=>{});};
 },[port,settings]);
 useEffect(()=>{
  if(!status?.capturing)return;
  const cancel=(event:PointerEvent)=>{if(!recorder.current?.contains(event.target as Node)){const token=captureToken.current;captureToken.current=null;void port.capture(false,token??undefined).catch(()=>{});}};
  document.addEventListener('pointerdown',cancel);return()=>document.removeEventListener('pointerdown',cancel);
 },[port,status?.capturing]);
 useEffect(()=>{
  const stop=()=>{const token=captureToken.current;captureToken.current=null;if(token)void port.capture(false,token).catch(()=>{});};
  const visibility=()=>{if(document.hidden)stop();};
  window.addEventListener('blur',stop);document.addEventListener('visibilitychange',visibility);
  return()=>{window.removeEventListener('blur',stop);document.removeEventListener('visibilitychange',visibility);};
 },[port]);
 async function capture(){
  if(starting)return;
  if(status?.capturing){const token=captureToken.current;captureToken.current=null;void port.capture(false,token??undefined).catch(()=>{});return;}
  const token=crypto.randomUUID();captureToken.current=token;setStarting(true);setError('');
  try{const value=await port.capture(true,token);if(!mounted.current||captureToken.current!==token){if(value.capturing&&value.captureToken===token)await port.capture(false,token);}else acceptStatus.current(value);}
  catch{if(mounted.current)setError('暂时无法录入，请重试。');if(captureToken.current===token)captureToken.current=null;}
  finally{if(mounted.current)setStarting(false);}
 }
 return <SettingsSection title="口述控制">
  <SettingsRow title="口述快捷键" description="点击右侧键位框，按下想使用的快捷键。" feedback={<>
   {status?.capturing&&<p className={s.captureHelp}>按下单键或组合键；Esc 或点击其他位置取消。</p>}
   {status?.pending&&<p role="status">已保存，本次口述结束后生效。</p>}{status?.error&&<InlineMessage status="error">{inputMessage(status.error)}</InlineMessage>}{status?.active&&!status.pending&&<p role="status">已启用：{shortcutLabel(status.active.binding)}</p>}
   {(error||state.error)&&<InlineMessage status="error">{error||state.error}</InlineMessage>}{state.error&&<Button variant="quiet" onClick={()=>void settings.load()}>重新读取</Button>}
  </>}><div className={s.recorderWrap} ref={recorder}><Button variant="quiet" className={s.shortcutRecorder} aria-label="更改快捷键" aria-pressed={status?.capturing??false} disabled={!state.ready||starting} onClick={()=>void capture()}>{status?.capturing?<span role="status">按下快捷键…</span>:draft?shortcutLabel(draft.binding).split(' + ').map((key,index)=><span className={s.recordedKey} key={key+index}>{index>0&&<span className={s.keyJoin}> + </span>}<Keycap>{key}</Keycap></span>):<span>正在读取</span>}</Button>{status?.capturing&&<Button variant="quiet" onClick={()=>void capture()}>取消录入</Button>}</div></SettingsRow>
  <SettingsRow title="录音方式" description={draft?.mode==='hold'?'按下开始，松开结束。':'按一下开始，再按一下结束。'}><SingleChoice label="录音方式" value={draft?.mode??'toggle'} disabled={!state.ready||status?.capturing} options={[{value:'toggle',label:'点按'},{value:'hold',label:'按住'}]} onValueChange={mode=>settings.update({mode:mode as ShortcutConfig['mode']})}/></SettingsRow>
 </SettingsSection>;
}
