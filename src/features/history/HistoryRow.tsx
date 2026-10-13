import {memo,useEffect,useRef,useState} from 'react';
import {ActionIcon,ActionMenu,Button,CopyFeedbackButton,Field,Select,IconButton,InlineMessage} from '../../ui';
import {SpeechError,speechError,type SpeechPort,type SpeechSnapshot} from '../../contracts/speech';
import {inputMessage} from '../../contracts/dictation';
import {historySelection,historyVersion,type DeliveryReport,type HistoryPort,type HistoryRecord,type HistorySummary,type PlaybackState} from '../../contracts/history';
import {modelExecutable,type ModelConfigPort,type ModelRecord} from '../../contracts/models';
import {speechMessages} from '../../locales/speech';
import s from './history.module.css';
export const duration=(ms:number)=>`${Math.floor(ms/60000)}:${String(Math.floor(ms/1000)%60).padStart(2,'0')}`;
const states:Record<string,string>={preparing:'准备中',recording:'录音中',processing:'识别中',success:'识别完成',failed:'识别失败',cancelled:'已取消',interrupted:'意外中断',delete_pending:'等待删除'};
const message=(code:string|null)=>code?speechMessages[speechError({code}).code]:'';
const audioLabels:Record<string,string>={prepared:'录音未完成',ready:'录音可用',delete_pending:'录音等待清理',expire_pending:'录音到期待清理',expired:'录音已到期',missing:'录音文件缺失',corrupt:'录音损坏'};
export const audioLabel=(r:Pick<HistoryRecord,'audio'|'durationMs'>)=>r.audio?.status==='missing'&&!r.audio.bytes&&!r.durationMs?'没有录下音频':r.audio?audioLabels[r.audio.status]:'未保留录音';
export type ExpandMode='text'|'versions'|'retry'|'delete'|'play';
type Props={deliveryRevision?:string;summary:HistorySummary;expanded:boolean;mode:ExpandMode;activation:number;port:HistoryPort;speech:SpeechPort;models:ModelConfigPort;snapshot:SpeechSnapshot;player:PlaybackState;playBusy:boolean;expand:(id:string,mode:ExpandMode)=>Promise<boolean>;collapse:()=>void;play:(action:'play'|'pause'|'resume'|'seek'|'stop',resource?:string,ms?:number)=>Promise<void>;changed:(id:string,removed?:boolean)=>void;onRecord:()=>void;copyFull:(id:string,attemptId:string|null,current:()=>boolean)=>Promise<boolean>};
export const HistoryRow=memo(function HistoryRow({deliveryRevision='',summary:r,expanded,mode,activation,port,speech,models,snapshot,player,playBusy,expand,collapse,play,changed,onRecord,copyFull}:Props){
 const display=historySelection(r);
 const [record,setRecord]=useState<HistoryRecord|null>(null),[version,setVersion]=useState(''),[loading,setLoading]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState(''),[retryModel,setRetryModel]=useState<ModelRecord|null>(null),[confirm,setConfirm]=useState<'retry'|'delete'|null>(null),[menuOpen,setMenuOpen]=useState(false),[overflow,setOverflow]=useState(false);
 const [deliveries,setDeliveries]=useState<DeliveryReport[]>([]);
 const alive=useRef(false),request=useRef(0),actionGeneration=useRef(0),working=useRef(false),preview=useRef<HTMLParagraphElement>(null),confirmRef=useRef<HTMLDivElement>(null),article=useRef<HTMLElement>(null);
 const selection=useRef({version,display,generation:0});useEffect(()=>{selection.current={version,display,generation:selection.current.generation+1};},[version,display]);
 const signature=JSON.stringify([r.revision,r.latestAttempt,r.latestStatus,r.audio]);
 useEffect(()=>{alive.current=true;return()=>{alive.current=false;};},[]);
 useEffect(()=>{const node=preview.current;if(!node)return;const check=()=>setOverflow(node.scrollHeight>node.clientHeight+1);check();const observer=typeof ResizeObserver==='undefined'?null:new ResizeObserver(check);observer?.observe(node);return()=>observer?.disconnect();},[r.preview]);
 useEffect(()=>{
  const token=++request.current;let disposed=false;
  if(!expanded){setRecord(null);setConfirm(null);setRetryModel(null);return;}
  setLoading(true);setError('');
  port.detail(r.id).then(value=>{if(disposed||token!==request.current)return;setRecord(value);setVersion(previous=>historyVersion(value,previous).text?previous:display??value.currentAttempt??value.attempts[0]?.id??'');setLoading(false);}).catch(e=>{if(!disposed&&token===request.current){setLoading(false);setError(message(speechError(e).code));}});
  return()=>{disposed=true;};
 },[expanded,port,r.id,display,signature]);
 useEffect(()=>{let disposed=false;if(!expanded){setDeliveries([]);return;}
  if(port.delivery)void port.delivery(r.id).then(rows=>{if(!disposed)setDeliveries(rows);}).catch(()=>{if(!disposed)setError('填入状态暂不可用。');});
  return()=>{disposed=true;};
 },[expanded,port,r.id,signature,deliveryRevision]);
 async function act(operation:()=>Promise<void>){if(working.current)return;const token=actionGeneration.current;working.current=true;setBusy(true);setError('');try{await operation();}catch(e){if(alive.current&&token===actionGeneration.current)setError(message(speechError(e).code));}finally{working.current=false;if(alive.current)setBusy(false);}}
 useEffect(()=>{
  const token=++actionGeneration.current;if(!expanded)return;
  if(mode==='delete')setConfirm('delete');
  if(mode==='retry')void act(async()=>{const overview=await models.overview();if(!alive.current||token!==actionGeneration.current)return;const model=overview.configs.find(c=>c.id===overview.defaults.find(d=>d.usage==='speech')?.configId);if(!model)throw new SpeechError('notConfigured');if(!modelExecutable(model,'speech'))throw new SpeechError('unsupported');setRetryModel(model);setConfirm('retry');});
  if(mode==='text'||mode==='play'||mode==='versions'){setConfirm(null);setRetryModel(null);}
  // Mode changes represent explicit user actions, not record refreshes.
 },[expanded,mode,activation,models]);
 useEffect(()=>{if(confirm)confirmRef.current?.querySelector<HTMLButtonElement>('button')?.focus();},[confirm]);
 function dismissConfirm(){setConfirm(null);setRetryModel(null);article.current?.querySelector<HTMLButtonElement>('button[aria-label="更多操作"]')?.focus();}
 async function copy(attemptId:string|null,detail=false){const token=selection.current.generation;try{return await copyFull(r.id,attemptId,()=>alive.current&&token===selection.current.generation&&(detail?selection.current.version===attemptId:selection.current.display===attemptId));}catch(e){if(alive.current&&speechError(e).code!=='cancelled')setError(message(speechError(e).code));throw e;}}
 async function playAudio(){await act(async()=>{if(!await expand(r.id,'play')||!alive.current)return;await play(player.resourceId===r.audio?.id?(player.playing?'pause':'resume'):'play',r.audio?.id);});}
 async function operation(action:'pause'|'resume'|'seek'|'stop',ms?:number){await act(async()=>{await play(action,undefined,ms);});}
 async function remove(){if(!record)return;await act(async()=>{try{await port.remove(record.id,record.revision);if(alive.current)changed(r.id,true);}catch(e){changed(r.id);throw e;}});}
 async function retry(){if(!record||!retryModel)return;await act(async()=>{await port.retry(record.id,retryModel.id,retryModel.revision);if(alive.current){setConfirm(null);changed(r.id);}});}
 const chosen=record?historyVersion(record,version):null;const attempt=chosen?.attempt;const processed=chosen?.processed;const fullText=chosen?.text??'';
 const delivery=deliveries.find(d=>d.attemptId===attempt?.id);const selected=record??r;
 const active=snapshot.recordId===r.id&&['connecting','recording','processing'].includes(snapshot.phase);
 const failure=r.latestStatus&&['failed','cancelled','interrupted'].includes(r.latestStatus);
 const reason=failure?(r.latestAttempt!==r.displayAttempt?'本次重识别'+(r.latestStatus==='cancelled'?'已取消':'未完成')+' · ':'')+(message(r.latestError)||states[r.latestStatus!]):'';
 const text=r.preview||reason||states[r.status]||'未识别到文字';
 return <article ref={article} role="listitem" tabIndex={0} data-record-id={r.id} data-expanded={expanded||menuOpen||undefined} className={s.record} aria-label={new Date(r.createdAt).toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit',hour12:false})+' 的记录'}>
  <div className={s.row}><time dateTime={new Date(r.createdAt).toISOString()}>{new Date(r.createdAt).toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit',hour12:false})}</time><div className={s.rowCopy}><p ref={preview} className={s.preview}>{text}</p>{r.matchedOldVersion&&<span className={s.meta}>匹配旧版本</span>}{r.preview&&reason&&<p className={s.meta}>{reason}</p>}{(r.truncated||overflow)&&!expanded&&<button className={s.expandLink} onClick={()=>void expand(r.id,'text')}>展开</button>}</div>
   <div className={s.rowActions}><CopyFeedbackButton key={display??'none'} compact disabled={!r.preview||busy} onCopy={()=>copy(display)}/>{r.audio?.status==='ready'?<IconButton label={player.resourceId===r.audio.id&&player.playing?'暂停录音':'播放录音'} disabled={active||playBusy||busy} onClick={()=>void playAudio()}><ActionIcon name={player.resourceId===r.audio.id&&player.playing?'pause':'play'}/></IconButton>:<span className={s.actionSpace}/>}
    <ActionMenu onOpenChange={setMenuOpen} items={[{label:'重新识别',disabled:busy||active||r.audio?.status!=='ready',onSelect:()=>void expand(r.id,'retry')},{label:'识别版本',disabled:busy,onSelect:()=>void expand(r.id,'versions')},{label:r.status==='delete_pending'?'重试删除':'删除记录',disabled:busy||active,onSelect:()=>void expand(r.id,'delete')}]}/>
   </div>
  </div>
  {error&&!expanded&&<div className={s.rowError}><InlineMessage status="error">{error}</InlineMessage></div>}
  {expanded&&<div className={s.detail}>
   <div className={s.detailHeading}><span className={s.meta}>{new Date(r.createdAt).toLocaleString('zh-CN')} · {duration(r.durationMs)}</span><IconButton label="收起记录" disabled={busy} onClick={collapse}><ActionIcon name="close"/></IconButton></div>
   {loading?<p role="status" className={s.meta}>正在读取…</p>:record&&<>
    {record.attempts.length+(record.processing?.filter(p=>p.state!=='running').length??0)>1&&<Field label="识别版本">{props=><Select {...props} label="识别版本" disabled={busy} value={version} onValueChange={setVersion} options={[...(record.processing?.filter(p=>p.state!=='running').map(p=>({value:'processed:'+p.id,label:p.mode==='native'?'口述结果':`整理结果 · ${p.configName}${p.state==='partial'?' · 部分保留原文':''}`}))??[]),...record.attempts.map(a=>({value:a.id,label:`${new Date(a.createdAt).toLocaleString('zh-CN')} · ${a.kind==='retry'?'重识别':'首次识别'} · ${states[a.status]??a.status}${a.id===record.currentAttempt?' · 当前':''} · 原文`}))]}/>}</Field>}
    <div className={s.fullText} role="region" aria-label="历史识别文字" tabIndex={0}>{fullText||message(attempt?.errorCode??null)||'未识别到文字'}</div>
    <div className={s.versionMeta}><span>{processed?.mode==='native'?'口述结果':`${processed?.configName??attempt?.configName} · ${processed?.modelId??attempt?.modelId}`}</span><CopyFeedbackButton key={version} compact disabled={!fullText||busy} onCopy={()=>copy(version||null,true)}/></div>
    {delivery&&<p role="status" className={s.meta}>{delivery.state==='confirmed'?'已填入':delivery.state==='unconfirmed'?'请确认输入结果':delivery.state==='copy'?'待复制':'填入未完成'}{delivery.error?' · '+inputMessage(delivery.error):''}</p>}
    {attempt?.text&&attempt.errorCode&&<InlineMessage status="error">{message(attempt.errorCode)}</InlineMessage>}
    {selected.audio?.status==='ready'?<div className={s.player}><div className={s.actions}><IconButton label={player.resourceId===selected.audio.id?(player.playing?'暂停':'继续'):'播放录音'} disabled={active||playBusy||busy} onClick={()=>void playAudio()}><ActionIcon name={player.resourceId===selected.audio.id&&player.playing?'pause':'play'}/></IconButton><input type="range" aria-label="录音播放位置" min={0} max={player.durationMs||1} value={Math.min(player.positionMs,player.durationMs)} disabled={player.resourceId!==selected.audio.id||busy||playBusy} onChange={e=>void operation('seek',Number(e.target.value))}/><span className={s.meta}>{duration(player.resourceId===selected.audio.id?player.positionMs:0)} / {duration(player.resourceId===selected.audio.id?player.durationMs:r.durationMs)}</span><IconButton label="停止播放" disabled={player.resourceId!==selected.audio.id||busy||playBusy} onClick={()=>void operation('stop')}><ActionIcon name="close"/></IconButton></div>{selected.audio.expiresAt&&<p className={s.meta}>录音保留至 {new Date(selected.audio.expiresAt).toLocaleDateString('zh-CN')}</p>}</div>:<div className={s.unavailable}><span className={s.meta}>{audioLabel(selected)}</span><Button variant="quiet" onClick={onRecord}>重新录音</Button></div>}
    {active&&<div className={s.actions}><span role="status" className={s.meta}>正在重新识别 · {duration(snapshot.durationMs)}</span><Button onClick={()=>void act(()=>speech.control(snapshot.taskId!,'cancel'))}>取消本次重识别</Button></div>}
   </>}
   {error&&<InlineMessage status="error">{error}</InlineMessage>}
   {confirm&&<div ref={confirmRef} className={s.confirm} role="group" aria-label={confirm==='delete'?'删除历史确认':'重识别确认'} onKeyDown={e=>{if(e.key==='Escape'&&!busy){dismissConfirm();e.stopPropagation();}}}>
    <h3>{confirm==='delete'?'删除这条记录？':'重新识别这份录音？'}</h3><p className={s.meta}>{confirm==='delete'?'文字、所有识别版本和录音将一起删除。':retryModel?`${retryModel.name} · ${retryModel.modelId}`:'正在读取模型…'}</p>
    {confirm==='retry'&&<p className={s.meta}>会发送这份录音，可能产生费用。</p>}
    <div className={s.actions}><Button disabled={busy} onClick={dismissConfirm}>取消</Button><Button variant="primary" loading={busy} disabled={!record||(confirm==='retry'&&!retryModel)} onClick={()=>void(confirm==='delete'?remove():retry())}>{confirm==='delete'?'确认删除':'开始识别'}</Button></div>
   </div>}
  </div>}
 </article>;
});
