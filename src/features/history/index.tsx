import {memo,useCallback,useEffect,useLayoutEffect,useRef,useState,useSyncExternalStore} from 'react';
import {useNavigate} from 'react-router-dom';
import {ActionIcon,Button,EmptyState,IconButton,InlineMessage,TextInput} from '../../ui';
import type {HistoryPort} from '../../contracts/history';
import type {SpeechPort,SpeechSnapshot} from '../../contracts/speech';
import {initialSpeech,speechError,SpeechError} from '../../contracts/speech';
import type {ModelConfigPort} from '../../contracts/models';
import {speechMessages} from '../../locales/speech';
import {HistoryFeedStore} from './feed';
import {HistoryRow,type ExpandMode} from './HistoryRow';
import {SaveSettings,type DepartureGuard} from './SaveSettings';
import {useHistoryPlayback} from './usePlayback';
import s from './history.module.css';
export {HistoryFeedStore} from './feed';
const quietSpeech=initialSpeech();
const day=(at:number)=>new Date(at).toLocaleDateString('zh-CN');
function dateLabel(at:number){const date=day(at),today=new Date(),yesterday=new Date(today);yesterday.setDate(today.getDate()-1);return date===day(today.getTime())?'今天':date===day(yesterday.getTime())?'昨天':date;}
export const History=memo(function History({deliveryRevision='',port,speech,models,snapshot,reading,visible=true,onChanged,onGuardChange}:{deliveryRevision?:string;port:HistoryPort;speech:SpeechPort;models:ModelConfigPort;snapshot:SpeechSnapshot;reading?:HistoryFeedStore;visible?:boolean;onChanged?:()=>void;onGuardChange?:(guard:DepartureGuard|null)=>void}){
 const navigate=useNavigate();
 const [own]=useState(()=>new HistoryFeedStore(port));const feed=reading??own;const state=useSyncExternalStore(feed.subscribe,feed.getSnapshot);
 const [mode,setMode]=useState<ExpandMode>('text'),[activation,setActivation]=useState(0),[settings,setSettings]=useState(false),[documentVisible,setDocumentVisible]=useState(()=>!document.hidden);
 const scroller=useRef<HTMLDivElement>(null),sentinel=useRef<HTMLDivElement>(null),guard=useRef<DepartureGuard|null>(null),opening=useRef(0),anchor=useRef<{id:string;offset:number}|null>(null);
 const alive=useRef(false),copyGeneration=useRef(0);
 useEffect(()=>{alive.current=true;return()=>{alive.current=false;};},[]);
 const copyFull=useCallback(async(id:string,attemptId:string|null,current:()=>boolean)=>{const token=++copyGeneration.current;const search=feed.getSnapshot().search;const full=await port.detail(id);if(!alive.current||token!==copyGeneration.current||search!==feed.getSnapshot().search||!current())return false;const attempt=full.attempts.find(a=>a.id===attemptId);if(!attempt?.text)throw new SpeechError('notFound');await speech.copy(attempt.text);return true;},[feed,port,speech]);
 const playback=useHistoryPlayback(port,visible&&documentVisible);const play=playback.control;const [emptyPlayer]=useState({resourceId:null,playing:false,positionMs:0,durationMs:0});
 const bindGuard=useCallback((value:DepartureGuard|null)=>{guard.current=value;onGuardChange?.(value);},[onGuardChange]);
 const recordAgain=useCallback(()=>{const action=()=>navigate('/');if(guard.current)guard.current(action);else action();},[navigate]);
 useEffect(()=>{const update=()=>setDocumentVisible(!document.hidden);document.addEventListener('visibilitychange',update);return()=>document.removeEventListener('visibilitychange',update);},[]);
 useEffect(()=>{if(visible&&documentVisible)void feed.refresh();},[feed,visible,documentVisible]);
 useLayoutEffect(()=>{if(scroller.current)scroller.current.scrollTop=feed.getScroll();},[feed]);
 const remember=useCallback(()=>{const node=scroller.current;if(!node)return;feed.setScroll(node.scrollTop);const items=node.querySelectorAll<HTMLElement>('[data-record-id]');const first=[...items].find(item=>item.offsetTop+item.offsetHeight>=node.scrollTop);anchor.current=first?{id:first.dataset.recordId!,offset:first.offsetTop-node.scrollTop}:null;},[feed]);
 useLayoutEffect(()=>{const node=scroller.current;if(!node||!anchor.current)return;const item=[...node.querySelectorAll<HTMLElement>('[data-record-id]')].find(r=>r.dataset.recordId===anchor.current?.id);if(item)node.scrollTop=item.offsetTop-anchor.current.offset;},[state.records]);
 useEffect(()=>{
  if(!visible||!documentVisible||!state.cursor||state.error)return;const node=sentinel.current,root=scroller.current;
  if(!node||!root||typeof IntersectionObserver==='undefined')return;
  const observer=new IntersectionObserver(entries=>{if(entries[0]?.isIntersecting)void feed.more();},{root,rootMargin:'0px 0px 240px 0px'});observer.observe(node);return()=>observer.disconnect();
 },[feed,visible,documentVisible,state.cursor,state.loadingMore,state.error]);
 const terminal=snapshot.saved||['failed','cancelled'].includes(snapshot.phase)?[snapshot.attemptId,snapshot.saved,snapshot.saveError,snapshot.error].join(':'):'';
 useEffect(()=>{if(terminal)void(async()=>{if(snapshot.recordId)await feed.update(snapshot.recordId);await feed.refresh();})();},[feed,terminal,snapshot.recordId]);
 const expand=useCallback(async(id:string,next:ExpandMode)=>{const token=++opening.current;if(feed.getSnapshot().expandedId!==id)await play('stop');if(!alive.current||token!==opening.current)return false;remember();setMode(next);setActivation(value=>value+1);feed.expand(id);return true;},[feed,play,remember]);
 const collapse=useCallback(()=>{opening.current++;void play('stop').catch(()=>{});feed.expand(null);},[feed,play]);
 const changed=useCallback((id:string,removed=false)=>{remember();if(removed){const node=scroller.current;if(node&&anchor.current?.id===id){const rows=[...node.querySelectorAll<HTMLElement>('[data-record-id]')];const index=rows.findIndex(row=>row.dataset.recordId===id);const next=rows[index+1]??rows[index-1];anchor.current=next?{id:next.dataset.recordId!,offset:next.offsetTop-node.scrollTop}:null;}feed.remove(id);}else void feed.update(id);onChanged?.();},[feed,remember,onChanged]);
 function toggleSettings(){const action=()=>setSettings(v=>!v);if(guard.current)guard.current(action);else action();}
 function latest(){anchor.current=null;if(scroller.current)scroller.current.scrollTop=0;feed.latest();}
 return <section className={s.page}>
  <header className={s.heading}><h1>历史记录</h1><Button aria-expanded={settings} aria-controls="history-settings" onClick={toggleSettings}>保存设置</Button></header>
  {settings&&<div id="history-settings"><SaveSettings speech={speech} onGuardChange={bindGuard}/></div>}
  <div className={s.toolbar}><TextInput aria-label="搜索识别文字" placeholder="搜索识别文字" maxLength={256} value={state.search} onChange={e=>{copyGeneration.current++;anchor.current=null;feed.search(e.target.value);if(scroller.current)scroller.current.scrollTop=0;}}/><span className={s.meta}>{state.total} 条</span><IconButton label="刷新历史" onClick={()=>void feed.refresh(undefined,true)}><ActionIcon name="retry"/></IconButton></div>
  {port.mode==='simulation'&&<p className={s.meta}>浏览器模拟 · 记录仅在本次运行中保留</p>}
  {state.newRecords&&<Button className={s.newRecords} onClick={latest}>有新记录，查看最新</Button>}
  <div ref={scroller} className={s.scroller} onScroll={remember} tabIndex={0} aria-label="历史记录列表">
   {state.loading&&!state.loaded?<p role="status" className={s.meta}>正在读取历史…</p>:state.records.length===0&&!state.error?<EmptyState title={state.search?'没有匹配的记录':'还没有历史记录'}>{!state.search&&<p className={s.meta}>完成一次录音后，文字会出现在这里。</p>}</EmptyState>:<div role="list" aria-label="识别记录" className={s.list}>{state.records.map((r,index)=>{const heading=index===0||day(state.records[index-1]!.createdAt)!==day(r.createdAt);return <div key={r.id}>{heading&&<h2 className={s.date}>{dateLabel(r.createdAt)}</h2>}<HistoryRow deliveryRevision={deliveryRevision} copyFull={copyFull} onRecord={recordAgain} activation={state.expandedId===r.id?activation:0} summary={r} expanded={state.expandedId===r.id} mode={mode} port={port} speech={speech} models={models} snapshot={snapshot.recordId===r.id?snapshot:quietSpeech} player={playback.state.resourceId===r.audio?.id?playback.state:emptyPlayer} playBusy={playback.busy} expand={expand} collapse={collapse} play={playback.control} changed={changed}/></div>;})}</div>}
   {state.error&&<div className={s.loadError}><InlineMessage status="error">{speechMessages[speechError({code:state.error}).code]}</InlineMessage><Button onClick={()=>void(state.loaded&&state.cursor?feed.more(true):feed.refresh(undefined,true))}>重试读取</Button></div>}
   <div ref={sentinel} className={s.end}>{state.loadingMore?<span role="status">正在读取更多…</span>:state.cursor&&!state.error?<Button variant="quiet" onClick={()=>void feed.more()}>加载更多</Button>:state.loaded&&state.records.length>0&&!state.error?<span>已到末尾</span>:null}</div>
  </div>
 </section>;
});
