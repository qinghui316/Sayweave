import {useEffect,useRef,useState} from 'react';
import type {LanguageSettingsPort,ProfileDetails,ProfileOverview,ProfileSummary} from '../../contracts/language';
import {Button,ActionIcon,InlineMessage} from '../../ui';
import s from './personalization.module.css';
const date=(value:number)=>value?new Date(value).toLocaleDateString('zh-CN'):'';
export function PersonalizationDashboard({port,visible}:{port:LanguageSettingsPort;visible:boolean}){
 const [view,setView]=useState<ProfileOverview|null>(null),[error,setError]=useState(''),[loading,setLoading]=useState(true);
 const [page,setPage]=useState(0),[refresh,setRefresh]=useState(0),[selected,setSelected]=useState<string|null>(null);
 const [detail,setDetail]=useState<ProfileDetails|null>(null),[detailError,setDetailError]=useState<{scope:string;version:number}|null>(null),[detailRefresh,setDetailRefresh]=useState(0);
 const cursors=useRef<(string|null)[]>([null]),overviewRequest=useRef(0),detailRequest=useRef(0);
 useEffect(()=>{
  if(!visible)return;let alive=true,release:(()=>void)|undefined,timer:ReturnType<typeof setTimeout>|undefined;
  const change=()=>{if(document.hidden)return;clearTimeout(timer);timer=setTimeout(()=>{if(alive){setLoading(true);setRefresh(v=>v+1);}},150);};
  void port.onChanged?.(change).then(stop=>{if(alive)release=stop;else stop();}).catch(()=>{});
  const restore=()=>{if(!document.hidden)change();};window.addEventListener('focus',restore);document.addEventListener('visibilitychange',restore);
  return()=>{alive=false;release?.();clearTimeout(timer);window.removeEventListener('focus',restore);document.removeEventListener('visibilitychange',restore);};
 },[port,visible]);
 useEffect(()=>{
  const token=++overviewRequest.current;let alive=true;if(!visible||document.hidden)return;
  void port.overview(cursors.current[page]??null).then(value=>{if(alive&&token===overviewRequest.current){setView(value);setError('');}}).catch(()=>{if(alive&&token===overviewRequest.current)setError('暂时无法读取学习数据，请重试。');}).finally(()=>{if(alive&&token===overviewRequest.current)setLoading(false);});
  return()=>{alive=false;};
 },[port,page,refresh,visible]);
 const summary=selected?(selected==='shared'?view?.shared:view?.applications.find(v=>v.scope===selected)):null;
 const version=summary?.version;
 useEffect(()=>{
  const token=++detailRequest.current;let alive=true;if(!visible||!selected||version===undefined)return;
  const scope=selected;
  void port.profile(scope).then(value=>{
   if(!alive||token!==detailRequest.current)return;
   if(value.scope!==scope||value.version<version)throw Error('stale profile');
   if(value.version>version){setRefresh(v=>v+1);return;}
   setDetail(value);setDetailError(null);
  }).catch(()=>{if(alive&&token===detailRequest.current)setDetailError({scope,version});});
  return()=>{alive=false;};
 },[port,selected,version,detailRefresh,visible]);
 function flip(next:number){overviewRequest.current++;detailRequest.current++;setSelected(null);setLoading(true);setPage(next);}
 function row(value:ProfileSummary){
  const expanded=selected===value.scope,failed=detailError?.scope===value.scope&&detailError.version===value.version;
  const current=detail?.scope===value.scope&&detail.version===value.version?detail:null;
  return <li key={value.scope} className={s.application}>
   <button type="button" className={s.applicationButton} disabled={loading} aria-expanded={expanded} aria-controls={'profile-'+value.scope} onClick={()=>setSelected(expanded?null:value.scope)}><span><strong>{value.name}</strong><span className={s.meta}>{value.preferenceCount?value.preferenceCount+'条表达习惯':value.version?'暂无表达习惯':'还在积累'}{value.updatedAt?' · '+date(value.updatedAt):''}</span></span><ActionIcon name="expand" size={16}/></button>
   {expanded&&<div id={'profile-'+value.scope} className={s.detail}>{failed?<><InlineMessage status="error">暂时无法读取应用资料。</InlineMessage><Button variant="quiet" onClick={()=>setDetailRefresh(v=>v+1)}>重试</Button></>:!current?<p className={s.meta}>正在读取…</p>:current.preferences.length?<ul>{current.preferences.map((item,index)=><li key={index}><p>{item.rule}</p>{item.context&&<span className={s.meta}>使用语境：{item.context}</span>}</li>)}</ul>:<p className={s.meta}>有更多使用记录后，这里会显示学到的表达习惯。</p>}</div>}
  </li>;
 }
 return <section aria-label="个性化数据" className={s.dashboard}>
  <div className={s.totals}>{[['已学表达习惯',view?.preferenceCount],['已记录应用',view?.applicationCount],['学习记录',view?.observationCount]].map(([label,value])=><div key={label}><strong>{value??'—'}</strong><span>{label}</span></div>)}</div>
  {error&&<div><InlineMessage status="error">{error}</InlineMessage><Button variant="quiet" onClick={()=>{setLoading(true);setRefresh(v=>v+1);}}>重新读取</Button></div>}
  {!view&&loading&&<p className={s.meta}>正在读取学习数据…</p>}
  {view&&<><h3>按应用学习</h3>{!view.applicationCount&&!view.shared.preferenceCount?<p className={s.meta}>使用一段时间后，这里会显示学到的表达习惯。</p>:<ul className={s.applications}>{view.shared.preferenceCount>0&&row(view.shared)}{view.applications.map(row)}</ul>}
  {(page>0||view.next)&&<div className={s.pagination}><Button disabled={loading||page===0} onClick={()=>flip(page-1)}>上一页</Button><span className={s.meta}>第 {page+1} 页</span><Button disabled={loading||!view.next} onClick={()=>{cursors.current[page+1]=view.next;flip(page+1);}}>下一页</Button></div>}</>}
 </section>;
}
