import {memo,useCallback,useEffect,useLayoutEffect,useRef,useState,useSyncExternalStore} from 'react';
import {useNavigate,useLocation} from 'react-router-dom';
import {ActionIcon,Button,EmptyState,IconButton,InlineMessage,ExpandableSearch} from '../../ui';
import {historyVersion,type HistoryPort} from '../../contracts/history';
import type {SpeechPort,SpeechSnapshot} from '../../contracts/speech';
import {initialSpeech,speechError,SpeechError} from '../../contracts/speech';
import type {ModelConfigPort} from '../../contracts/models';
import {speechMessages} from '../../locales/speech';
import {HistoryFeedStore} from './feed';
import {HistoryRow,type ExpandMode} from './HistoryRow';
import {SaveSettings} from './SaveSettings';
import type {RecordingPreferences} from '../../contracts/speech';
import type {SettingStore} from '../../settings/store';
import {useHistoryPlayback} from './usePlayback';
import s from './history.module.css';
export {HistoryFeedStore} from './feed';
const quietSpeech=initialSpeech();
const day=(at:number)=>new Date(at).toLocaleDateString('zh-CN');
function dateLabel(at:number){const date=day(at),today=new Date(),yesterday=new Date(today);yesterday.setDate(today.getDate()-1);return date===day(today.getTime())?'今天':date===day(yesterday.getTime())?'昨天':date;}
export const History=memo(function History({deliveryRevision='',contentRevision=0,port,speech,models,snapshot,reading,visible=true,onChanged,recordingSettings}:{deliveryRevision?:string;contentRevision?:number;port:HistoryPort;speech:SpeechPort;models:ModelConfigPort;snapshot:SpeechSnapshot;reading?:HistoryFeedStore;visible?:boolean;onChanged?:()=>void;recordingSettings:SettingStore<RecordingPreferences>}){
 const navigate=useNavigate(),location=useLocation();
 useEffect(()=>{if(new URLSearchParams(location.search).get('settings')==='save')setSettings(true);},[location.search]);
 const [own]=useState(()=>new HistoryFeedStore(port));const feed=reading??own;const state=useSyncExternalStore(feed.subscribe,feed.getSnapshot);
 const [mode,setMode]=useState<ExpandMode>('text'),[activation,setActivation]=useState(0),[settings,setSettings]=useState(false),[documentVisible,setDocumentVisible]=useState(()=>!document.hidden);
 const scroller=useRef<HTMLDivElement>(null),opening=useRef(0),anchor=useRef<{id:string;offset:number}|null>(null);
 const [navigationError,setNavigationError]=useState(''),[navigating,setNavigating]=useState(false);const navigationPending=useRef(false);
 const alive=useRef(false),copyGeneration=useRef(0);
 useEffect(()=>{alive.current=true;return()=>{alive.current=false;};},[]);
 const copyFull=useCallback(async(id:string,attemptId:string|null,current:()=>boolean)=>{if(navigationPending.current)return false;const token=++copyGeneration.current;const epoch=feed.getSnapshot().epoch;const full=await port.detail(id);if(!alive.current||token!==copyGeneration.current||epoch!==feed.getSnapshot().epoch||!current())return false;const selected=historyVersion(full,attemptId);if(!selected.text)throw new SpeechError('notFound');await speech.copy(selected.text);return true;},[feed,port,speech]);
 const playback=useHistoryPlayback(port,visible&&documentVisible);const play=playback.control;const [emptyPlayer]=useState({resourceId:null,playing:false,positionMs:0,durationMs:0});
 const recordAgain=useCallback(()=>navigate('/'),[navigate]);
 useEffect(()=>{const update=()=>setDocumentVisible(!document.hidden);document.addEventListener('visibilitychange',update);return()=>document.removeEventListener('visibilitychange',update);},[]);
 useEffect(()=>{if(visible&&documentVisible)void feed.refresh();},[feed,visible,documentVisible,contentRevision]);
 useLayoutEffect(()=>{if(scroller.current)scroller.current.scrollTop=feed.getScroll();},[feed]);
 const remember=useCallback(()=>{const node=scroller.current;if(!node)return;feed.setScroll(node.scrollTop);const items=node.querySelectorAll<HTMLElement>('[data-record-id]');const first=[...items].find(item=>item.offsetTop+item.offsetHeight>=node.scrollTop);anchor.current=first?{id:first.dataset.recordId!,offset:first.offsetTop-node.scrollTop}:null;},[feed]);
 useLayoutEffect(()=>{const node=scroller.current;if(!node||!anchor.current)return;const item=[...node.querySelectorAll<HTMLElement>('[data-record-id]')].find(r=>r.dataset.recordId===anchor.current?.id);if(item)node.scrollTop=item.offsetTop-anchor.current.offset;},[state.records]);
 const terminal=snapshot.saved||['failed','cancelled'].includes(snapshot.phase)?[snapshot.attemptId,snapshot.saved,snapshot.saveError,snapshot.error].join(':'):'';
 useEffect(()=>{if(terminal)void feed.refresh();},[feed,terminal,snapshot.recordId]);
 const expand=useCallback(async(id:string,next:ExpandMode)=>{if(navigationPending.current)return false;const token=++opening.current,epoch=feed.getSnapshot().epoch;if(feed.getSnapshot().expandedId!==id)await play('stop');if(!alive.current||token!==opening.current||epoch!==feed.getSnapshot().epoch||feed.getSnapshot().loadingMore)return false;remember();setMode(next);setActivation(value=>value+1);feed.expand(id);return true;},[feed,play,remember]);
 const collapse=useCallback(()=>{opening.current++;void play('stop').catch(()=>{});feed.expand(null);},[feed,play]);
 const changed=useCallback((id:string,removed=false)=>{remember();if(removed){const node=scroller.current;if(node&&anchor.current?.id===id){const rows=[...node.querySelectorAll<HTMLElement>('[data-record-id]')];const index=rows.findIndex(row=>row.dataset.recordId===id);const next=rows[index+1]??rows[index-1];anchor.current=next?{id:next.dataset.recordId!,offset:next.offsetTop-node.scrollTop}:null;}feed.remove(id);}else void feed.update(id);onChanged?.();},[feed,remember,onChanged]);
 function toggleSettings(){setSettings(v=>!v);}
 async function navigatePage(action:()=>Promise<void>){
  if(navigationPending.current||state.loadingMore)return;
  const epoch=feed.getSnapshot().epoch;navigationPending.current=true;setNavigating(true);setNavigationError('');copyGeneration.current++;opening.current++;
  try{await play('stop');if(!alive.current||epoch!==feed.getSnapshot().epoch)return;anchor.current=null;await action();}
  catch{if(alive.current)setNavigationError('暂时无法停止播放，请重试。');}
  finally{navigationPending.current=false;if(alive.current)setNavigating(false);}
 }
 function latest(){void navigatePage(()=>feed.latest());}
 useLayoutEffect(()=>{anchor.current=null;if(scroller.current)scroller.current.scrollTop=feed.getScroll();},[feed,state.page,state.search]);
 return <section className={s.page}>
  <header className={s.heading}><h1>历史记录</h1><Button aria-expanded={settings} aria-controls="history-settings" onClick={toggleSettings}>历史保存设置</Button></header>
  {settings&&<div id="history-settings"><SaveSettings settings={recordingSettings}/></div>}
  <div className={s.toolbar}><ExpandableSearch anchor="left" label="搜索识别文字" value={state.search} onValueChange={value=>{copyGeneration.current++;anchor.current=null;feed.search(value);if(scroller.current)scroller.current.scrollTop=0;}}/><div className={s.toolbarActions}><span className={s.meta}>{state.total} 条</span><IconButton label="刷新历史" disabled={navigating||state.loadingMore} onClick={latest}><ActionIcon name="retry"/></IconButton></div></div>
  {state.newRecords&&<Button className={s.newRecords} onClick={latest}>有新记录，查看最新</Button>}
  <div ref={scroller} className={s.scroller} onScroll={remember} tabIndex={0} aria-label="历史记录列表">
   {state.loading&&!state.loaded?<p role="status" className={s.meta}>正在读取历史…</p>:state.records.length===0&&!state.error?<EmptyState title={state.search?'没有匹配的记录':'还没有历史记录'}>{!state.search&&<p className={s.meta}>完成一次录音后，文字会出现在这里。</p>}</EmptyState>:<div role="list" aria-label="识别记录" className={s.list}>{state.records.map((r,index)=>{const heading=index===0||day(state.records[index-1]!.createdAt)!==day(r.createdAt);return <div key={r.id}>{heading&&<h2 className={s.date}>{dateLabel(r.createdAt)}</h2>}<HistoryRow deliveryRevision={deliveryRevision} copyFull={copyFull} onRecord={recordAgain} activation={state.expandedId===r.id?activation:0} summary={r} expanded={state.expandedId===r.id} mode={mode} port={port} speech={speech} models={models} snapshot={snapshot.recordId===r.id?snapshot:quietSpeech} player={playback.state.resourceId===r.audio?.id?playback.state:emptyPlayer} playBusy={playback.busy} expand={expand} collapse={collapse} play={playback.control} changed={changed}/></div>;})}</div>}
   {state.error&&<div className={s.loadError}><InlineMessage status="error">{speechMessages[speechError({code:state.error}).code]}</InlineMessage><Button onClick={()=>void navigatePage(()=>feed.retry())}>重试读取</Button></div>}

  </div>
  {navigationError&&<InlineMessage status="error">{navigationError}</InlineMessage>}
  <nav className={s.pagination} aria-label="历史分页"><Button disabled={navigating||state.loading||state.loadingMore||state.page<=1} onClick={()=>void navigatePage(feed.previous)}>上一页</Button><span aria-live="polite">第 {state.page} 页 · {state.total} 条</span><Button disabled={navigating||state.loading||state.loadingMore||!state.cursor} onClick={()=>void navigatePage(feed.next)}>下一页</Button>{state.loadingMore&&<span role="status">正在翻页…</span>}</nav>
 </section>;
});
