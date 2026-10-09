import { useEffect, useRef, useState, useId, type FormEvent } from 'react';
import { dictionaryError, validateDictionaryInput, type DictionaryPort, type DictionaryKind, type DictionaryRecord, type DictionaryPage, type DictionaryError } from '../../contracts/dictionary';
import { Button, Dialog, EmptyState, Field, InlineMessage, TextInput,SingleChoice,ActionMenu,ActionIcon,IconButton } from '../../ui';
import { zh } from '../../locales/zh-CN';
import s from './dictionary.module.css';
import {ToggleSetting} from '../../settings/ToggleSetting';
import type {SettingStore} from '../../settings/store';
import type {ProcessingSettings} from '../../contracts/processing';
const t=zh.dictionary;
type Editor={kind:DictionaryKind;record:DictionaryRecord|null; original:string; category:string; replacement:string};
export function Dictionary({port,automaticTerms}:{port:DictionaryPort;automaticTerms:SettingStore<ProcessingSettings>}) {
  const [kind,setKind]=useState<DictionaryKind>('term');
  const [source,setSource]=useState<'all'|'manual'|'automatic'>('all');
  const [searchOpen,setSearchOpen]=useState(false);const searchId=useId(),searchBox=useRef<HTMLDivElement|null>(null),searchField=useRef<HTMLInputElement|null>(null);
  const [search,setSearch]=useState(''); const [page,setPage]=useState(1); const [refresh,setRefresh]=useState(0);
  const [result,setResult]=useState<DictionaryPage>({records:[],total:0,page:1});
  const [loading,setLoading]=useState(true); const [error,setError]=useState<DictionaryError|null>(null);
  const [editor,setEditor]=useState<Editor|null>(null); const [deleting,setDeleting]=useState<DictionaryRecord|null>(null);
  const [supplement,setSupplement]=useState(false);
  const [formError,setFormError]=useState<DictionaryError|null>(null); const [busy,setBusy]=useState(false); const [feedback,setFeedback]=useState('');
  const pageElement=useRef<HTMLElement|null>(null),focusBack=useRef<HTMLElement|null>(null),rows=useRef(new Map<string,HTMLLIElement>());
  const restoreFocus=()=>focusBack.current?.isConnected?focusBack.current:pageElement.current?.querySelector<HTMLElement>('[data-dictionary-add]')??null;
  const generation=useRef(0); const mutation=useRef(0); const inFlight=useRef(false);
  useEffect(()=>()=>{mutation.current++;},[]);
  useEffect(()=>{let disposed=false;let release:(()=>void)|undefined;void port.onChanged?.(()=>setRefresh(v=>v+1)).then(stop=>{if(disposed)stop();else release=stop;}).catch(()=>{});return()=>{disposed=true;release?.();};},[port]);
  useEffect(()=>{
    const current=++generation.current; let disposed=false;
    const timer=setTimeout(()=>{
      setLoading(true);setError(null);
      (async()=>{
        const status=await port.status(refresh>0);
        if(status.state==='unavailable') throw dictionaryError(status.error);
        const data=await port.list({kind,search,page,...(kind==='term'&&source!=='all'?{source}:{})});
        if(!disposed && current===generation.current) {setResult(data);setLoading(false);}
      })().catch(e=>{if(!disposed && current===generation.current){setError(dictionaryError(e));setLoading(false);}});
    },search?180:0);
    return ()=>{disposed=true;clearTimeout(timer);};
  },[port,kind,search,page,refresh,source]);
  useEffect(()=>{if(!searchOpen)return;const close=(event:PointerEvent)=>{if(!search&& !searchBox.current?.contains(event.target as Node))setSearchOpen(false);};document.addEventListener('pointerdown',close);return()=>document.removeEventListener('pointerdown',close);},[searchOpen,search]);
  const reload=()=>{setPage(1);setRefresh(v=>v+1);};
  function open(record:DictionaryRecord|null,target?:HTMLElement) {if(!editor)focusBack.current=target??(record?rows.current.get(record.id)?.querySelector<HTMLElement>('[aria-haspopup="menu"]')??null:pageElement.current?.querySelector('[data-dictionary-add]')??null);setSupplement(false);setFormError(null);setFeedback('');setEditor({kind:record?.kind??kind,record,original:record?.original??'',category:record?.category??'',replacement:record?.replacement??''});}
  function close() {if(!inFlight.current){mutation.current++;setEditor(null);setDeleting(null);setFormError(null);}}
  async function save(event:FormEvent) {
    event.preventDefault(); if(!editor||inFlight.current) return;
    const token=++mutation.current;inFlight.current=true;setBusy(true);setFormError(null);
    try {
      const input=validateDictionaryInput({kind:editor.kind,original:editor.original,category:editor.kind==='term'?editor.category||null:null,replacement:editor.kind==='correction'?editor.replacement:null});
      if(editor.record) await port.update(input,editor.record.id,editor.record.revision); else await port.create(input);
      if(token===mutation.current){setEditor(null);setFeedback(t.saved);reload();}
    } catch(e) {if(token===mutation.current)setFormError(dictionaryError(e));}
    finally {inFlight.current=false;if(token===mutation.current)setBusy(false);}
  }
  async function remove() {
    if(!deleting||inFlight.current)return;
    const token=++mutation.current;inFlight.current=true;setBusy(true);setFormError(null);
    try {await port.delete(deleting.kind,deleting.id,deleting.revision);if(token===mutation.current){setDeleting(null);setFeedback(t.deleted);reload();}}
    catch(e){if(token===mutation.current)setFormError(dictionaryError(e));}
    finally{inFlight.current=false;if(token===mutation.current)setBusy(false);}
  }
  async function readExisting(id:string) {
    if(inFlight.current)return;
    const token=++mutation.current;inFlight.current=true;setBusy(true);
    try{const record=await port.get(kind,id);if(token===mutation.current)open(record);}
    catch(e){if(token===mutation.current)setFormError(dictionaryError(e));}
    finally{inFlight.current=false;if(token===mutation.current)setBusy(false);}
  }
  return <section ref={pageElement} className={s.page}>
    <header className={s.heading}><div><h1>{kind==='term'?'词典':'纠错映射'}</h1>{port.persistence==='memory'&&<p className={s.meta}>{t.memory}</p>}</div><div className={s.headerActions}><ToggleSetting compact store={automaticTerms} label="自动入词" description="从真实文字纠正中学习正确词形，关闭后已有词语仍可使用。"/><ActionMenu label="词典更多操作" items={[{label:kind==='term'?'纠错映射':'返回词典',onSelect:()=>{setKind(kind==='term'?'correction':'term');setPage(1);setSearch('');setSearchOpen(false);setLoading(true);setFeedback('');}}]}/><Button data-dictionary-add aria-label={kind==='term'?'添加词语':'添加纠错'} variant="primary" className={s.add} onClick={()=>open(null)} disabled={!!error||loading}>{kind==='term'?'新词':'新纠错'}</Button></div></header>
    <div className={s.filters}><div className={s.filterGroup}>{kind==='term'?<SingleChoice value={source} label="词条来源" onValueChange={value=>{setSource(value as typeof source);setPage(1);setLoading(true);}} options={[{value:'all',label:'所有'},{value:'automatic',label:'自动添加'},{value:'manual',label:'手动添加'}]}/>:<span className={s.meta}>纠正特定的识别写法</span>}{result.total>0&&<span className={s.meta}>{t.total(result.total)}</span>}</div><div ref={searchBox} className={s.searchSlot}>{!searchOpen?<IconButton label="搜索词语" aria-expanded={false} aria-controls={searchId} className={s.searchTrigger} onClick={()=>setSearchOpen(true)}><ActionIcon name="search" size={16}/></IconButton>:<div id={searchId} className={s.searchBox}><div className={s.searchContents}><ActionIcon name="search" size={16}/><TextInput ref={searchField} autoFocus aria-label={t.search} placeholder="搜索" value={search} maxLength={256} onKeyDown={event=>{if(event.key==='Escape'){event.preventDefault();event.stopPropagation();setSearch('');setPage(1);setSearchOpen(false);requestAnimationFrame(()=>searchBox.current?.querySelector('button')?.focus());}}} onChange={e=>{setSearch(e.target.value);setPage(1);setLoading(true);}}/></div></div>}</div></div>
    {feedback&&<InlineMessage status="success">{feedback}</InlineMessage>}
    {loading?<p role="status" className={s.empty}>{t.loading}</p>:error?<div className={s.empty}><InlineMessage status="error">{t.errors[error.code]}</InlineMessage><Button onClick={reload}>{t.retry}</Button></div>:result.records.length===0?<EmptyState title={search?t.noMatches:t.empty} description={search?undefined:t.emptyDescription}/>:<ul className={s.words} aria-label="词语列表">{result.records.map(r=><li key={r.id} ref={node=>{if(node)rows.current.set(r.id,node);else rows.current.delete(r.id);}}><button type="button" className={s.wordButton} aria-label={'编辑词语 '+r.original} onClick={event=>open(r,event.currentTarget)}><span className={r.source==='automatic'?s.automatic:s.manual} role="img" aria-label={r.source==='automatic'?'自动添加':'手动添加'}><ActionIcon name={r.source==='automatic'?'automatic':'manual'} size={14}/></span><span className={s.word} title={r.original}>{r.original}{r.kind==='correction'&&<span className={s.meta}> → {r.replacement}</span>}</span></button><ActionMenu label={'更多操作 '+r.original} items={[{label:'编辑',onSelect:()=>open(r)},{label:'删除',onSelect:()=>{focusBack.current=rows.current.get(r.id)?.querySelector<HTMLElement>('[aria-haspopup="menu"]')??null;setFormError(null);setDeleting(r);}}]}/></li>)}</ul>}
    {!loading&&!error&&result.total>50&&<footer className={s.pagination}><span className={s.meta}>{t.page(result.page,result.total)}</span><Button disabled={result.page<=1} onClick={()=>{setLoading(true);setPage(result.page-1);}}>{t.previous}</Button><Button disabled={result.page>=Math.ceil(result.total/50)} onClick={()=>{setLoading(true);setPage(result.page+1);}}>{t.next}</Button></footer>}
    <Dialog restoreFocus={restoreFocus} open={editor!==null} onOpenChange={v=>{if(!v)close();}} title={editor?.record?t.editTitle:t.newTitle} description={t.formDescription} className={s.modal}>
      {editor&&<form className={s.form} onSubmit={save}><h2>{editor.record?t.editTitle:t.newTitle}</h2>
        <Field label={editor.kind==='term'?t.original:t.heard}>{p=><TextInput {...p} disabled={busy} value={editor.original} onChange={e=>setEditor({...editor,original:e.target.value})}/>}</Field>
        {editor.kind==='correction'?<Field label={t.replacement}>{p=><TextInput {...p} disabled={busy} value={editor.replacement} onChange={e=>setEditor({...editor,replacement:e.target.value})}/>}</Field>:<><Button variant="quiet" aria-expanded={supplement} onClick={()=>setSupplement(v=>!v)}>补充设置</Button>{supplement&&<Field label={t.category}>{p=><TextInput {...p} disabled={busy} value={editor.category} onChange={e=>setEditor({...editor,category:e.target.value})}/>}</Field>}</>}
        {formError&&<InlineMessage status="error">{t.errors[formError.code]}</InlineMessage>}
        {formError?.existingId&&<Button disabled={busy} onClick={()=>readExisting(formError.existingId!)}>{t.viewExisting}</Button>}
        {formError?.code==='conflict'&&editor.record&&<Button disabled={busy} onClick={()=>readExisting(editor.record!.id)}>{t.reloadConflict}</Button>}
        <footer className={s.actions}><Button disabled={busy} onClick={close}>{t.cancel}</Button><Button variant="primary" type="submit" loading={busy}>{t.save}</Button></footer>
      </form>}
    </Dialog>
    <Dialog restoreFocus={restoreFocus} open={deleting!==null} onOpenChange={v=>{if(!v)close();}} title={t.removeTitle} description={t.removeDescription} className={s.modal}>
      <div className={s.form}><h2>{t.removeTitle}</h2><p>{deleting?.original}</p><p className={s.meta}>{t.removeDescription}</p>{formError&&<InlineMessage status="error">{t.errors[formError.code]}</InlineMessage>}<footer className={s.actions}><Button disabled={busy} onClick={close}>{t.cancel}</Button><Button variant="primary" loading={busy} onClick={remove}>{t.remove}</Button></footer></div>
    </Dialog>
  </section>;
}

