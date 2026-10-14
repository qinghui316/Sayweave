import { useCallback,useEffect, useRef, useState, type FormEvent } from 'react';
import { dictionaryError, validateDictionaryInput, type DictionaryPort, type DictionaryRecord, type DictionaryPage, type DictionaryError } from '../../contracts/dictionary';
import { Button, Dialog, EmptyState, Field, InlineMessage, TextInput,SingleChoice,ActionMenu,ActionIcon,ExpandableSearch } from '../../ui';
import { zh } from '../../locales/zh-CN';
import s from './dictionary.module.css';
import {ToggleSetting} from '../../settings/ToggleSetting';
import type {SettingStore} from '../../settings/store';
import type {ProcessingSettings} from '../../contracts/processing';
const t=zh.dictionary;
type Editor={record:DictionaryRecord|null; original:string; category:string};
export function Dictionary({port,automaticTerms,onGuardChange}:{port:DictionaryPort;automaticTerms:SettingStore<ProcessingSettings>;onGuardChange?:(guard:((action:()=>void)=>void)|null)=>void}) {
  const [source,setSource]=useState<'all'|'manual'|'automatic'>('all');
  const [search,setSearch]=useState(''); const [page,setPage]=useState(1); const [refresh,setRefresh]=useState(0);
  const [result,setResult]=useState<DictionaryPage>({records:[],total:0,page:1});
  const [loading,setLoading]=useState(true); const [error,setError]=useState<DictionaryError|null>(null);
  const [editor,setEditor]=useState<Editor|null>(null); const [deleting,setDeleting]=useState<DictionaryRecord|null>(null);
  const [supplement,setSupplement]=useState(false);
  const [formError,setFormError]=useState<DictionaryError|null>(null); const [busy,setBusy]=useState(false);
  const pageElement=useRef<HTMLElement|null>(null),focusBack=useRef<HTMLElement|null>(null),rows=useRef(new Map<string,HTMLLIElement>());
  const restoreFocus=()=>focusBack.current?.isConnected?focusBack.current:pageElement.current?.querySelector<HTMLElement>('[data-dictionary-add]')??null;
  const generation=useRef(0); const mutation=useRef(0); const inFlight=useRef(false);
  const [leaving,setLeaving]=useState(false);const nextAction=useRef<(()=>void)|null>(null);
  const guard=useCallback((action:()=>void)=>{if(inFlight.current)return;if(editor){nextAction.current=action;setLeaving(true);}else action();},[editor]);
  useEffect(()=>{onGuardChange?.(guard);return()=>onGuardChange?.(null);},[guard,onGuardChange]);
  useEffect(()=>()=>{mutation.current++;},[]);
  useEffect(()=>{let disposed=false;let release:(()=>void)|undefined;void port.onChanged?.(()=>setRefresh(v=>v+1)).then(stop=>{if(disposed)stop();else release=stop;}).catch(()=>{});return()=>{disposed=true;release?.();};},[port]);
  useEffect(()=>{
    const current=++generation.current; let disposed=false;
    const timer=setTimeout(()=>{
      setLoading(true);setError(null);
      (async()=>{
        const status=await port.status(refresh>0);
        if(status.state==='unavailable') throw dictionaryError(status.error);
        const data=await port.list({kind:'term',search,page,...(source!=='all'?{source}:{})});
        if(!disposed && current===generation.current) {setResult(data);setLoading(false);}
      })().catch(e=>{if(!disposed && current===generation.current){setError(dictionaryError(e));setLoading(false);}});
    },search?180:0);
    return ()=>{disposed=true;clearTimeout(timer);};
  },[port,search,page,refresh,source]);
  const reload=()=>{setPage(1);setRefresh(v=>v+1);};
  function open(record:DictionaryRecord|null,target?:HTMLElement) {if(!editor)focusBack.current=target??(record?rows.current.get(record.id)?.querySelector<HTMLElement>('[aria-haspopup="menu"]')??null:pageElement.current?.querySelector('[data-dictionary-add]')??null);setSupplement(false);setFormError(null);setEditor({record,original:record?.original??'',category:record?.category??''});}
  function close() {if(!inFlight.current){mutation.current++;setEditor(null);setDeleting(null);setFormError(null);}}
  async function save(event?:FormEvent) {
    event?.preventDefault(); if(!editor||inFlight.current) return false;
    const token=++mutation.current;inFlight.current=true;setBusy(true);setFormError(null);
    try {
      const input=validateDictionaryInput({kind:'term',original:editor.original,category:editor.category||null,replacement:null});
      if(editor.record) await port.update(input,editor.record.id,editor.record.revision); else await port.create(input);
      if(token===mutation.current){setEditor(null);reload();return true;}
      return false;
    } catch(e) {if(token===mutation.current){setFormError(dictionaryError(e));requestAnimationFrame(()=>pageElement.current?.querySelector<HTMLInputElement>('input')?.focus());}return false;}
    finally {inFlight.current=false;if(token===mutation.current)setBusy(false);}
  }
  async function remove() {
    if(!deleting||inFlight.current)return;
    const token=++mutation.current;inFlight.current=true;setBusy(true);setFormError(null);
    try {await port.delete('term',deleting.id,deleting.revision);if(token===mutation.current){setDeleting(null);reload();}}
    catch(e){if(token===mutation.current)setFormError(dictionaryError(e));}
    finally{inFlight.current=false;if(token===mutation.current)setBusy(false);}
  }
  async function readExisting(id:string) {
    if(inFlight.current)return;
    const token=++mutation.current;inFlight.current=true;setBusy(true);
    try{const record=await port.get('term',id);if(token===mutation.current)open(record);}
    catch(e){if(token===mutation.current)setFormError(dictionaryError(e));}
    finally{inFlight.current=false;if(token===mutation.current)setBusy(false);}
  }
  return <section ref={pageElement} className={s.page}>
    <Dialog confirmation open={leaving} onOpenChange={open=>{if(!open){setLeaving(false);nextAction.current=null;}}} title="更新前保存词语" description="保存后继续更新，或取消并返回编辑。"><div className={s.actions}><Button variant="primary" disabled={busy} onClick={()=>{const action=nextAction.current;nextAction.current=null;setLeaving(false);void save().then(saved=>{if(saved)action?.();});}}>保存</Button><Button disabled={busy} onClick={()=>{setLeaving(false);nextAction.current=null;}}>取消</Button></div></Dialog>
    <header className={s.heading}><div><h1>词典</h1>{port.persistence==='memory'&&<p className={s.meta}>{t.memory}</p>}</div><div className={s.headerActions}><ToggleSetting compact store={automaticTerms} label="自动入词" description="从真实文字纠正中学习正确词形，关闭后已有词语仍可使用。"/><Button data-dictionary-add aria-label="添加词语" variant="primary" className={s.add} onClick={()=>open(null)} disabled={!!error||loading}>新词</Button></div></header>
    <div className={s.filters}><div className={s.filterGroup}><SingleChoice value={source} label="词条来源" onValueChange={value=>{setSource(value as typeof source);setPage(1);setLoading(true);}} options={[{value:'all',label:'所有'},{value:'automatic',label:'自动添加'},{value:'manual',label:'手动添加'}]}/>{result.total>0&&<span className={s.meta}>{t.total(result.total)}</span>}</div><ExpandableSearch value={search} label={t.search} triggerLabel="搜索词语" placeholder="搜索" onValueChange={value=>{setSearch(value);setPage(1);setLoading(true);}}/></div>
    {loading?<p role="status" className={s.empty}>{t.loading}</p>:error?<div className={s.empty}><InlineMessage status="error">{t.errors[error.code]}</InlineMessage><Button onClick={reload}>{t.retry}</Button></div>:result.records.length===0?<EmptyState title={search?t.noMatches:t.empty} description={search?undefined:t.emptyDescription}/>:<ul className={s.words} aria-label="词语列表">{result.records.map(r=><li key={r.id} ref={node=>{if(node)rows.current.set(r.id,node);else rows.current.delete(r.id);}}><button type="button" className={s.wordButton} aria-label={'编辑词语 '+r.original} onClick={event=>open(r,event.currentTarget)}><span className={r.source==='automatic'?s.automatic:s.manual} role="img" aria-label={r.source==='automatic'?'自动添加':'手动添加'}><ActionIcon name={r.source==='automatic'?'automatic':'manual'} size={14}/></span><span className={s.word} title={r.original}>{r.original}</span></button><ActionMenu label={'更多操作 '+r.original} items={[{label:'编辑',onSelect:()=>open(r)},{label:'删除',onSelect:()=>{focusBack.current=rows.current.get(r.id)?.querySelector<HTMLElement>('[aria-haspopup="menu"]')??null;setFormError(null);setDeleting(r);}}]}/></li>)}</ul>}
    {!loading&&!error&&result.total>50&&<footer className={s.pagination}><span className={s.meta}>{t.page(result.page,result.total)}</span><Button disabled={result.page<=1} onClick={()=>{setLoading(true);setPage(result.page-1);}}>{t.previous}</Button><Button disabled={result.page>=Math.ceil(result.total/50)} onClick={()=>{setLoading(true);setPage(result.page+1);}}>{t.next}</Button></footer>}
    <Dialog restoreFocus={restoreFocus} open={editor!==null} onOpenChange={v=>{if(!v)close();}} title={editor?.record?t.editTitle:t.newTitle} description={t.formDescription} className={s.modal}>
      {editor&&<form className={s.form} onSubmit={save}><h2>{editor.record?t.editTitle:t.newTitle}</h2>
        <Field label={t.original}>{p=><TextInput {...p} disabled={busy} value={editor.original} onChange={e=>setEditor({...editor,original:e.target.value})}/>}</Field>
        <><Button variant="quiet" aria-expanded={supplement} onClick={()=>setSupplement(v=>!v)}>补充设置</Button>{supplement&&<Field label={t.category}>{p=><TextInput {...p} disabled={busy} value={editor.category} onChange={e=>setEditor({...editor,category:e.target.value})}/>}</Field>}</>
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

