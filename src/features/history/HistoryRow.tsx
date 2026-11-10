import {memo,useEffect,useId,useRef,useState} from 'react';
import {ActionIcon,Button,CopyFeedbackButton,IconButton} from '../../ui';
import {speechError,type SpeechSnapshot} from '../../contracts/speech';
import {speechMessages} from '../../locales/speech';
import {historySelection,historyVersion,type DeliveryReport,type HistoryPort,type HistoryRecord,type HistorySummary,type PlaybackState} from '../../contracts/history';
import {inputMessage} from '../../contracts/dictation';
import s from './history.module.css';

export const duration=(ms:number)=>`${Math.floor(ms/60000)}:${String(Math.floor(ms/1000)%60).padStart(2,'0')}`;
const states:Record<string,string>={preparing:'准备中',recording:'录音中',processing:'识别中',success:'识别完成',failed:'识别失败',cancelled:'已取消',interrupted:'意外中断',delete_pending:'等待删除'};
const message=(code:string|null)=>code?speechMessages[speechError({code}).code]:'';
const audioLabels:Record<string,string>={prepared:'录音未完成',ready:'录音可用',delete_pending:'录音等待清理',expire_pending:'录音到期待清理',expired:'录音已到期',missing:'录音文件缺失',corrupt:'录音损坏'};
export const audioLabel=(r:Pick<HistoryRecord,'audio'|'durationMs'>)=>r.audio?.status==='missing'&&!r.audio.bytes&&!r.durationMs?'没有录下音频':r.audio?audioLabels[r.audio.status]:'未保留录音';
export const detailUnavailable='无法读取这条历史记录。';

function HistoryText({summary:r,display,deliveryRevision,port,id}:{summary:HistorySummary;display:string|null;deliveryRevision:string;port:HistoryPort;id:string}){
 const [detail,setDetail]=useState<HistoryRecord|null|undefined>(undefined);
 const [deliveries,setDeliveries]=useState<DeliveryReport[]>([]);
 useEffect(()=>{
  let disposed=false;
  void port.detail(r.id).then(value=>{if(!disposed)setDetail(value);},()=>{if(!disposed)setDetail(null);});
  return()=>{disposed=true;};
 },[port,r.id]);
 useEffect(()=>{
  let disposed=false;
  void port.delivery?.(r.id).then(rows=>{if(!disposed)setDeliveries(rows);},()=>{if(!disposed)setDeliveries([]);});
  return()=>{disposed=true;};
 },[port,r.id,deliveryRevision]);
 const chosen=detail?historyVersion(detail,display):null;
 const attempt=chosen?.attempt,processed=chosen?.processed;
 const translation=attempt?.purpose==='translation'?(processed??detail?.processing?.find(p=>p.attemptId===attempt.id&&!p.text)):undefined;
 const translationFailure=translation?(translation.errorCode==='cancelled'||translation.state==='cancelled'?'翻译已取消。':translation.errorCode==='emptyText'?'未生成译文。':translation.errorCode||translation.state==='failed'||translation.state==='interrupted'?'翻译未完成。':''):'';
 const delivery=deliveries.find(d=>d.attemptId===attempt?.id);
 return <div id={id} className={s.detail}>
  <p className={s.meta}>{new Date(r.createdAt).toLocaleString('zh-CN')} · {duration(r.durationMs)}</p>
  {detail===undefined?<p role="status" className={s.meta}>正在读取…</p>:!detail||(!chosen?.text&&r.preview)?<p role="alert" className={s.meta}>{detailUnavailable}</p>:<>
   <div className={s.fullText} role="region" aria-label="历史识别文字" tabIndex={0}>{chosen?.text||message(attempt?.errorCode??null)||'未识别到文字'}</div>
   {translationFailure&&<p className={s.meta}>{translationFailure}</p>}
   {attempt&&<p className={s.meta}>{attempt.purpose==='translation'?`口述翻译 · ${attempt.targetLanguage??'目标未冻结'} · `:''}{processed?.mode==='native'?'口述结果':`${processed?.configName??attempt.configName} · ${processed?.modelId??attempt.modelId}`}</p>}
   {delivery&&<p role="status" className={s.meta}>{delivery.state==='confirmed'?'已填入':delivery.state==='unconfirmed'?'请确认输入结果':delivery.state==='copy'?'待复制':'填入未完成'}{delivery.error?' · '+inputMessage(delivery.error):''}</p>}
  </>}
 </div>;
}

export type ExpandMode='text'|'delete';
type Props={deliveryRevision?:string;summary:HistorySummary;expanded:boolean;mode:ExpandMode;port:HistoryPort;snapshot:SpeechSnapshot;player:PlaybackState;playBusy:boolean;expand:(id:string,mode:ExpandMode)=>boolean;collapse:()=>void;play:(action:'play'|'pause'|'resume'|'stop',resource?:string)=>Promise<void>;changed:(id:string,removed?:boolean)=>void;copyFull:(id:string,attemptId:string|null,current:()=>boolean)=>Promise<boolean>};
export const HistoryRow=memo(function HistoryRow({deliveryRevision='',summary:r,expanded,mode,port,snapshot,player,playBusy,expand,collapse,play,changed,copyFull}:Props){
 const display=historySelection(r),detailId=useId();
 const showText=expanded&&mode==='text',showDelete=expanded&&mode==='delete';
 const resourceId=r.audio?.status==='ready'?r.audio.id:null;
 const audioActive=resourceId!==null&&player.resourceId===resourceId;
 const active=snapshot.recordId===r.id&&['connecting','recording','processing'].includes(snapshot.phase);
 const [audioFailure,setAudioFailure]=useState<{id:string;text:string}|null>(null);
 const [copyFailure,setCopyFailure]=useState<{id:string|null;text:string}|null>(null);
 const [deleteError,setDeleteError]=useState('');
 const [deleting,setDeleting]=useState(false),[overflow,setOverflow]=useState(false);
 const alive=useRef(false),selection=useRef(0),deletion=useRef<number|null>(null);
 const preview=useRef<HTMLParagraphElement>(null),article=useRef<HTMLElement>(null),confirm=useRef<HTMLDivElement>(null);
 useEffect(()=>{alive.current=true;return()=>{alive.current=false;};},[]);
 useEffect(()=>{selection.current++;},[display]);
 useEffect(()=>{if(showDelete)confirm.current?.querySelector('button')?.focus();},[showDelete]);
 useEffect(()=>{const node=preview.current;if(!node)return;const check=()=>setOverflow(node.scrollHeight>node.clientHeight+1);check();const observer=typeof ResizeObserver==='undefined'?null:new ResizeObserver(check);observer?.observe(node);return()=>observer?.disconnect();},[r.preview]);

 async function copy(){
  setCopyFailure(null);
  const token=selection.current;
  const current=()=>alive.current&&selection.current===token;
  try{return await copyFull(r.id,display,current);}
  catch(e){if(current()&&speechError(e).code!=='cancelled')setCopyFailure({id:display,text:speechError(e).code==='copyFailed'?message('copyFailed'):detailUnavailable});throw e;}
 }
 async function playAudio(){
  if(!resourceId)return;
  setAudioFailure(null);
  try{await play(audioActive?(player.playing?'pause':'resume'):'play',resourceId);}
  catch(e){
   const code=speechError(e).code;
   if(alive.current&&code!=='cancelled')setAudioFailure({id:resourceId,text:code==='fileUnavailable'||code==='notFound'?'录音文件缺失或无法读取。':code==='busy'?message(code):'无法播放这段录音。'});
  }
 }
 function askDelete(){
  if(expand(r.id,'delete')){deletion.current=r.revision;setDeleteError('');}
 }
 function dismissDelete(){collapse();article.current?.querySelector<HTMLButtonElement>('button[data-history-delete]')?.focus();}
 async function remove(){
  const revision=deletion.current;
  if(deleting||playBusy||revision===null)return;
  setDeleting(true);setDeleteError('');
  try{
   if(audioActive)await play('stop');
   await port.remove(r.id,revision);
   if(alive.current)changed(r.id,true);
  }catch(e){
   if(alive.current){changed(r.id);setDeleteError(message(speechError(e).code));}
  }finally{if(alive.current)setDeleting(false);}
 }
 const failure=r.latestStatus&&['failed','cancelled','interrupted'].includes(r.latestStatus);
 const reason=failure?(r.latestAttempt!==r.displayAttempt?'本次重识别'+(r.latestStatus==='cancelled'?'已取消':'未完成')+' · ':'')+(message(r.latestError)||states[r.latestStatus!]):'';
 const text=r.preview||reason||states[r.status]||'未识别到文字';
 const audioError=audioFailure?.id===resourceId?audioFailure.text:'';
 const copyError=copyFailure?.id===display?copyFailure.text:'';
 const audioNotice=audioError||(['missing','corrupt'].includes(r.audio?.status??'')?audioLabel(r):'');
 const playLabel=audioActive?(player.playing?'暂停录音':'继续播放'):'播放录音';

 return <article ref={article} role="listitem" tabIndex={0} data-record-id={r.id} data-expanded={expanded||undefined} className={s.record} aria-label={new Date(r.createdAt).toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit',hour12:false})+' 的记录'}>
  <div className={s.row}>
   <time dateTime={new Date(r.createdAt).toISOString()}>{new Date(r.createdAt).toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit',hour12:false})}</time>
   <div className={s.rowCopy}>
    <p ref={preview} className={s.preview}>{text}</p>
    {r.matchedOldVersion&&<span className={s.meta}>匹配旧版本</span>}
    {r.preview&&reason&&<p className={s.meta}>{reason}</p>}
    {(r.truncated||overflow||showText)&&<button type="button" className={s.expandLink} aria-expanded={showText} aria-controls={showText?detailId:undefined} disabled={deleting} onClick={()=>showText?collapse():expand(r.id,'text')}>{showText?'收起':'展开'}</button>}
    {audioNotice&&<p className={s.meta} role={audioError?'alert':undefined}>{audioNotice}</p>}
    {copyError&&<p className={s.meta} role="alert">{copyError}</p>}
   </div>
   <div className={s.rowActions}>
    <CopyFeedbackButton key={display??'none'} compact disabled={!r.preview||deleting} onCopy={copy}/>
    {resourceId?<span className={s.playAction}><IconButton label={playLabel} disabled={active||playBusy||deleting} onClick={()=>void playAudio()}><ActionIcon name={audioActive&&player.playing?'pause':'play'}/></IconButton></span>:<span className={s.actionSpace}/>}
    <IconButton data-history-delete="" label={r.status==='delete_pending'?'重试删除':'删除记录'} aria-expanded={showDelete} disabled={active||deleting} onClick={askDelete}><ActionIcon name="delete"/></IconButton>
   </div>
  </div>
  {showText&&<HistoryText key={JSON.stringify([display,r.revision,r.latestAttempt,r.latestStatus])} summary={r} display={display} deliveryRevision={deliveryRevision} port={port} id={detailId}/>}
  {showDelete&&<div ref={confirm} className={s.confirm} role="group" aria-label="删除历史确认" onKeyDown={e=>{if(e.key==='Escape'&&!deleting){e.stopPropagation();dismissDelete();}}}>
   <h3>删除这条记录？</h3><p className={s.meta}>{r.audio?'这条记录的文字和录音将一起删除。':'这条记录的文字将被删除。'}</p>
   {deleteError&&<p role="alert" className={s.meta}>{deleteError}</p>}
   <div className={s.actions}><Button disabled={deleting} onClick={dismissDelete}>取消</Button><Button variant="primary" loading={deleting} disabled={playBusy} onClick={()=>void remove()}>确认删除</Button></div>
  </div>}
 </article>;
});
