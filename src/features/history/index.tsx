import {useEffect,useRef,useState} from 'react';
import {Link} from 'react-router-dom';
import {Button,CopyFeedbackButton,Dialog,EmptyState,Field,InlineMessage,TextInput} from '../../ui';
import {historyText,type HistoryPort,type HistoryRecord,type HistoryPage,type PlaybackState} from '../../contracts/history';
import {speechError,type RecordingPreferences,type SpeechPort,type SpeechSnapshot} from '../../contracts/speech';
import {modelExecutable,type ModelConfigPort,type ModelRecord} from '../../contracts/models';
import {speechMessages} from '../../locales/speech';
import s from './history.module.css';
const statusLabel:Record<string,string>={preparing:'准备中',recording:'录音中',processing:'处理中',success:'识别完成',failed:'识别失败',cancelled:'已取消',interrupted:'意外中断',delete_pending:'等待删除'};
const audioLabel:Record<string,string>={prepared:'录音未完成',ready:'录音可用',delete_pending:'录音等待清理',expire_pending:'录音到期待清理',expired:'录音已到期',missing:'录音文件缺失',corrupt:'录音损坏'};
const recordAudioLabel=(r:HistoryRecord)=>r.audio?.status==='missing'&&r.audio.bytes===0&&r.durationMs===0?'没有录下音频':r.audio?audioLabel[r.audio.status]:'未永久保留录音';
const time=(ms:number)=>new Date(ms).toLocaleString('zh-CN');
const duration=(ms:number)=>`${Math.floor(ms/60000)}:${String(Math.floor(ms/1000)%60).padStart(2,'0')}`;
export function History({port,speech,models,snapshot}:{port:HistoryPort;speech:SpeechPort;models:ModelConfigPort;snapshot:SpeechSnapshot}){
 const [search,setSearch]=useState(''),[page,setPage]=useState(1),[refresh,setRefresh]=useState(0);
 const [result,setResult]=useState<HistoryPage>({records:[],total:0,page:1,pageSize:50}),[loading,setLoading]=useState(true),[error,setError]=useState('');
 const [record,setRecord]=useState<HistoryRecord|null>(null),[version,setVersion]=useState(''),[deleting,setDeleting]=useState(false),[retryModel,setRetryModel]=useState<ModelRecord|null>(null);
 const [busy,setBusy]=useState(false),[notice,setNotice]=useState(''),[prefs,setPrefs]=useState<RecordingPreferences|null>(null);
 const [player,setPlayer]=useState<PlaybackState>({resourceId:null,playing:false,positionMs:0,durationMs:0});
 const generation=useRef(0),alive=useRef(false),mutation=useRef(0),inFlight=useRef(false);
 const playbackGeneration=useRef(0);
 useEffect(()=>{alive.current=true;return()=>{alive.current=false;};},[]);
 useEffect(()=>{
  const current=++generation.current;let stopped=false;
  const timer=setTimeout(()=>{setLoading(true);setError('');(async()=>{
   const state=await port.status(refresh>0);if(state.state!=='ready')throw speechError({code:state.state==='preparing'?'preparing':'unavailable'});
   const data=await port.list(search,page);if(!stopped&&current===generation.current){setResult(data);setLoading(false);}
  })().catch(e=>{if(!stopped&&current===generation.current){setError(speechMessages[speechError(e).code]);setLoading(false);}});},search?180:0);
  return()=>{stopped=true;clearTimeout(timer);};
 },[port,search,page,refresh,snapshot.lifecycle]);
 useEffect(()=>{let stopped=false;speech.preferences().then(p=>{if(!stopped)setPrefs(p);}).catch(()=>{});return()=>{stopped=true;};},[speech,refresh]);
 useEffect(()=>{let stopped=false;const timer=setInterval(()=>{if(inFlight.current)return;const token=++playbackGeneration.current;port.playback('status').then(p=>{if(!stopped&&token===playbackGeneration.current)setPlayer(p);}).catch(()=>{});},250);return()=>{stopped=true;clearInterval(timer);};},[port]);
 const selectedId=record?.id;
 useEffect(()=>{if(selectedId&&snapshot.recordId===selectedId&&['completed','failed','cancelled','idle'].includes(snapshot.phase)){let stopped=false;port.detail(selectedId).then(r=>{if(!stopped)setRecord(r);}).catch(()=>{});return()=>{stopped=true;};}},[port,selectedId,snapshot.lifecycle,snapshot.recordId,snapshot.phase]);
 const reload=()=>{setPage(1);setRefresh(v=>v+1);};
 async function action(work:()=>Promise<void>){if(inFlight.current)return;inFlight.current=true;setBusy(true);setError('');const token=++mutation.current;try{await work();}catch(e){if(alive.current&&mutation.current===token)setError(speechMessages[speechError(e).code]);}finally{inFlight.current=false;if(alive.current&&mutation.current===token)setBusy(false);}}
 async function open(id:string){await action(async()=>{const r=await port.detail(id);if(alive.current){setRecord(r);setVersion(r.currentAttempt??r.attempts[0]?.id??'');setNotice('');}});}
 async function deleteRecord(){if(!record)return;await action(async()=>{try{await port.remove(record.id,record.revision);}catch(e){setRecord(await port.detail(record.id));throw e;}if(alive.current){setRecord(null);setDeleting(false);setNotice('记录与录音已删除。');reload();}});}
 async function prepareRetry(){await action(async()=>{await models.status();const v=await models.overview();const c=v.configs.find(c=>c.id===v.defaults.find(d=>d.usage==='speech')?.configId);if(!c)throw speechError({code:'notConfigured'});if(!modelExecutable(c,'speech'))throw speechError({code:'unsupported'});if(alive.current)setRetryModel(c);});}
 async function retry(){if(!record||!retryModel)return;await action(async()=>{await port.retry(record.id,retryModel.id,retryModel.revision);if(alive.current){setRetryModel(null);setNotice('正在重识别；取消只停止本次重试，原记录保留。');}});}
 async function play(actionName:'play'|'pause'|'resume'|'seek'|'stop',ms?:number){await action(async()=>{const token=++playbackGeneration.current;try{const p=actionName==='play'?await port.play(record!.audio!.id):await port.playback(actionName,ms);if(alive.current&&token===playbackGeneration.current)setPlayer(p);}catch(e){if(record){const updated=await port.detail(record.id);if(alive.current)setRecord(updated);}throw e;}});}
 async function savePreferences(){if(!prefs)return;await action(async()=>{const p=await speech.savePreferences(prefs);if(alive.current){setPrefs(p);setNotice('保存策略已更新，仅影响后续录音。');}});}
 const attempt=record?.attempts.find(a=>a.id===version)??record?.attempts.find(a=>a.id===record.currentAttempt)??record?.attempts[0];
 const active=snapshot.recordId===record?.id&&['connecting','recording','processing'].includes(snapshot.phase);
 return <section className={s.page}>
  <header className={s.heading}><h1>历史记录</h1><p>原始识别文字与每次重识别版本；录音到期后文字仍保留。</p><Link to="/">返回首页</Link></header>
  {port.mode==='simulation'&&<InlineMessage>浏览器模拟：记录只存在内存中，不证明磁盘持久化、播放或云端识别。</InlineMessage>}
  <details className={s.preferences}><summary>历史与录音保存</summary>{prefs&&<div className={s.settings}>
   <label><input type="checkbox" checked={prefs.historyEnabled} disabled={busy} onChange={e=>setPrefs({...prefs,historyEnabled:e.target.checked})}/>保存文字历史</label>
   <label><input type="checkbox" checked={prefs.audioEnabled} disabled={busy||!prefs.historyEnabled} onChange={e=>setPrefs({...prefs,audioEnabled:e.target.checked})}/>保留录音</label>
   <Field label="新录音保留期限">{p=><select {...p} value={prefs.retentionDays??'forever'} disabled={busy||!prefs.historyEnabled||!prefs.audioEnabled} onChange={e=>setPrefs({...prefs,retentionDays:e.target.value==='forever'?null:Number(e.target.value) as 7|30|90})}><option value="7">7 天</option><option value="30">30 天</option><option value="90">90 天</option><option value="forever">永久</option></select>}</Field>
   <p>关闭历史时不永久保存录音。修改不追溯改变旧录音期限。</p><Button loading={busy} onClick={()=>void savePreferences()}>保存策略</Button>
  </div>}</details>
  <div className={s.toolbar}><TextInput aria-label="搜索识别文字" placeholder="搜索识别文字" maxLength={256} value={search} onChange={e=>{setSearch(e.target.value);setPage(1);}}/><span>{result.total} 条</span><Button onClick={reload}>刷新</Button></div>
  {error&&<InlineMessage status="error">{error}</InlineMessage>}{notice&&<InlineMessage>{notice}</InlineMessage>}
  {loading?<p role="status">正在读取历史…</p>:error&&!record?<Button onClick={reload}>重试读取</Button>:result.records.length===0?<EmptyState title={search?'没有匹配的记录':'还没有识别记录'} description="在首页完成一次真实口述，开启保存后会出现在这里。"/>:
  <div className={s.list}>{result.records.map(r=><button className={s.record} key={r.id} onClick={()=>void open(r.id)}><strong>{historyText(r)||'无识别文字'}</strong><span>{time(r.createdAt)} · {duration(r.durationMs)} · {statusLabel[r.status]??r.status}{r.audio?' · '+recordAudioLabel(r):''}</span></button>)}</div>}
  {result.total>0&&<footer className={s.actions}><span>{result.page} / {Math.ceil(result.total/50)}</span><Button disabled={result.page<=1} onClick={()=>setPage(result.page-1)}>上一页</Button><Button disabled={result.page>=Math.ceil(result.total/50)} onClick={()=>setPage(result.page+1)}>下一页</Button></footer>}
  <Dialog open={record!==null} onOpenChange={open=>{if(!open&&!inFlight.current){setRecord(null);setDeleting(false);setRetryModel(null);setError('');}}} title="识别详情" description="查看原始文字、版本及录音" className={s.dialog}>
   {record&&<div className={s.detail}><h2>识别详情</h2><p>{time(record.createdAt)} · {duration(record.durationMs)}</p>
    <Field label="识别版本">{p=><select {...p} value={attempt?.id??''} onChange={e=>setVersion(e.target.value)}>{record.attempts.map((a,i)=><option key={a.id} value={a.id}>{time(a.createdAt)} · {a.kind==='retry'?'重识别':'首次识别'} · {statusLabel[a.status]??a.status}{a.id===record.currentAttempt?' · 当前':''}{i===0?' · 最新':''}</option>)}</select>}</Field>
    <p className={s.meta}>{attempt?.configName} · {attempt?.modelId} · {attempt?.serviceHost}</p>
    <div className={s.text} role="region" aria-label="历史识别文字" tabIndex={0}>{attempt?.text||'此版本没有可用文字。'}</div>
    {attempt?.errorCode&&<InlineMessage status="error">{speechMessages[speechError({code:attempt.errorCode}).code]}</InlineMessage>}
    <CopyFeedbackButton key={attempt?.id} disabled={!attempt?.text} onCopy={()=>speech.copy(attempt?.text??'')}/>
    <div className={s.player}><h3>录音</h3><p>{recordAudioLabel(record)}{record.audio?.status==='ready'&&record.audio.expiresAt?' · 到期 '+time(record.audio.expiresAt):''}</p>
     {record.audio?.status==='ready'&&<><div className={s.actions}><Button disabled={busy||active} onClick={()=>void play(player.resourceId===record.audio?.id?(player.playing?'pause':'resume'):'play')}>{player.resourceId===record.audio.id?(player.playing?'暂停':'继续'):'播放录音'}</Button><Button disabled={player.resourceId!==record.audio.id} onClick={()=>void play('stop')}>停止</Button><span>{duration(player.resourceId===record.audio.id?player.positionMs:0)} / {duration(record.audio.bytes/32)}</span></div><input type="range" aria-label="录音播放位置" min="0" max={record.audio.bytes/32} value={player.resourceId===record.audio.id?Math.min(player.positionMs,record.audio.bytes/32):0} disabled={player.resourceId!==record.audio.id||busy} onChange={e=>void play('seek',Number(e.target.value))}/></>}
    </div>
    {active&&<InlineMessage>正在重识别 · {Math.floor(snapshot.durationMs/1000)} 秒 <Button onClick={()=>void speech.control(snapshot.taskId!,'cancel').catch(e=>setError(speechMessages[speechError(e).code]))}>取消本次重识别</Button></InlineMessage>}
    <div className={s.actions}><Button disabled={busy||active||!(record.audio?.status==='ready'||(!record.audio&&snapshot.recordId===record.id&&snapshot.canRetry))} onClick={()=>void prepareRetry()}>重新识别</Button><Button disabled={busy||active} onClick={()=>{setDeleting(true);setError('');}}>删除记录</Button></div>
    {retryModel&&<div className={s.confirm} role="alertdialog" aria-label="重识别确认"><h3>重新识别此录音？</h3><p>{retryModel.name} · {retryModel.modelId}<br/>{new URL(retryModel.baseUrl).host}</p><p>会上传这份录音并产生真实请求，可能产生费用。旧成功结果保留。</p><Button loading={busy} onClick={()=>void retry()}>确认重识别</Button><Button disabled={busy} onClick={()=>setRetryModel(null)}>取消</Button></div>}
    {deleting&&<div className={s.confirm} role="alertdialog" aria-label="删除历史确认"><h3>删除记录和所有识别版本？</h3><p>同时清理当前录音文件；升级备份中的副本不会因此清除。</p><Button loading={busy} onClick={()=>void deleteRecord()}>确认删除</Button><Button disabled={busy} onClick={()=>setDeleting(false)}>取消</Button></div>}
    {error&&<InlineMessage status="error">{error}</InlineMessage>}{notice&&<InlineMessage>{notice}</InlineMessage>}
   </div>}
  </Dialog>
 </section>;
}
