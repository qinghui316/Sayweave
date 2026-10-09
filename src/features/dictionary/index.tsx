import { useEffect, useRef, useState, type FormEvent } from 'react';
import { dictionaryError, validateDictionaryInput, type DictionaryPort, type DictionaryKind, type DictionaryRecord, type DictionaryPage, type DictionaryError } from '../../contracts/dictionary';
import { Button, Dialog, EmptyState, Field, InlineMessage, Tabs, TextInput,SingleChoice } from '../../ui';
import { zh } from '../../locales/zh-CN';
import s from './dictionary.module.css';
const t=zh.dictionary;
type Editor={record:DictionaryRecord|null; original:string; category:string; replacement:string};
export function Dictionary({port}:{port:DictionaryPort}) {
  const [kind,setKind]=useState<DictionaryKind>('term');
  const [source,setSource]=useState<'all'|'manual'|'automatic'>('all');
  const [search,setSearch]=useState(''); const [page,setPage]=useState(1); const [refresh,setRefresh]=useState(0);
  const [result,setResult]=useState<DictionaryPage>({records:[],total:0,page:1});
  const [loading,setLoading]=useState(true); const [error,setError]=useState<DictionaryError|null>(null);
  const [editor,setEditor]=useState<Editor|null>(null); const [deleting,setDeleting]=useState<DictionaryRecord|null>(null);
  const [formError,setFormError]=useState<DictionaryError|null>(null); const [busy,setBusy]=useState(false); const [feedback,setFeedback]=useState('');
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
  const reload=()=>{setPage(1);setRefresh(v=>v+1);};
  function open(record:DictionaryRecord|null) {setFormError(null);setFeedback('');setEditor({record,original:record?.original??'',category:record?.category??'',replacement:record?.replacement??''});}
  function close() {if(!inFlight.current){mutation.current++;setEditor(null);setDeleting(null);setFormError(null);}}
  async function save(event:FormEvent) {
    event.preventDefault(); if(!editor||inFlight.current) return;
    const token=++mutation.current;inFlight.current=true;setBusy(true);setFormError(null);
    try {
      const input=validateDictionaryInput({kind,original:editor.original,category:kind==='term'?editor.category||null:null,replacement:kind==='correction'?editor.replacement:null});
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
  return <section className={s.page}>
    <header className={s.heading}><h1>{t.title}</h1>{port.persistence==='memory'&&<p className={s.meta}>{t.memory}</p>}</header>
    <Tabs value={kind} label={t.tabs} onValueChange={v=>{setKind(v as DictionaryKind);setPage(1);setSearch('');setLoading(true);setFeedback('');}} options={[{value:'term',label:t.terms},{value:'correction',label:t.corrections}]}>
      <div className={s.panel}><div className={s.toolbar}><TextInput aria-label={t.search} placeholder={t.search} value={search} maxLength={256} onChange={e=>{setSearch(e.target.value);setPage(1);setLoading(true);}}/><span className={s.meta}>{t.total(result.total)}</span><Button variant="primary" onClick={()=>open(null)} disabled={!!error||loading}>{kind==='term'?t.addTerm:t.addCorrection}</Button></div>
        {kind==='term'&&<div className={s.sourceFilter}><SingleChoice value={source} label="词条来源" onValueChange={value=>{setSource(value as typeof source);setPage(1);setLoading(true);}} options={[{value:'all',label:'全部'},{value:'manual',label:'手动'},{value:'automatic',label:'自动'}]}/></div>}
        {feedback&&<InlineMessage status="success">{feedback}</InlineMessage>}
        {loading?<p role="status" className={s.empty}>{t.loading}</p>:error?<div className={s.empty}><InlineMessage status="error">{t.errors[error.code]}</InlineMessage><Button onClick={reload}>{t.retry}</Button></div>:result.records.length===0?<EmptyState title={search?t.noMatches:t.empty} description={search?undefined:t.emptyDescription}/>:<div className={s.tableWrap}><table><thead><tr><th>{kind==='term'?t.original:t.heard}</th><th>{kind==='term'?t.categoryColumn:t.replacement}</th><th>{t.actions}</th></tr></thead><tbody>{result.records.map(r=><tr key={r.id}><td>{r.original}</td><td className={kind==='term'?s.meta:undefined}>{kind==='term'?r.category??'—':r.replacement}</td><td><div className={s.actions}><Button variant="quiet" aria-label={`${t.edit} ${r.original}`} onClick={()=>open(r)}>{t.edit}</Button><Button variant="quiet" aria-label={`${t.remove} ${r.original}`} onClick={()=>{setFormError(null);setDeleting(r);}}>{t.remove}</Button></div></td></tr>)}</tbody></table></div>}
        {!loading&&!error&&result.total>0&&<footer className={s.pagination}><span className={s.meta}>{t.page(result.page,result.total)}</span><Button disabled={result.page<=1} onClick={()=>{setLoading(true);setPage(result.page-1);}}>{t.previous}</Button><Button disabled={result.page>=Math.ceil(result.total/50)} onClick={()=>{setLoading(true);setPage(result.page+1);}}>{t.next}</Button></footer>}
      </div>
    </Tabs>
    <Dialog open={editor!==null} onOpenChange={v=>{if(!v)close();}} title={editor?.record?t.editTitle:t.newTitle} description={t.formDescription} className={s.modal}>
      {editor&&<form className={s.form} onSubmit={save}><h2>{editor.record?t.editTitle:t.newTitle}</h2>
        <Field label={kind==='term'?t.original:t.heard}>{p=><TextInput {...p} disabled={busy} value={editor.original} onChange={e=>setEditor({...editor,original:e.target.value})}/>}</Field>
        <Field label={kind==='term'?t.category:t.replacement}>{p=><TextInput {...p} disabled={busy} value={kind==='term'?editor.category:editor.replacement} onChange={e=>setEditor({...editor,[kind==='term'?'category':'replacement']:e.target.value})}/>}</Field>
        {formError&&<InlineMessage status="error">{t.errors[formError.code]}</InlineMessage>}
        {formError?.existingId&&<Button disabled={busy} onClick={()=>readExisting(formError.existingId!)}>{t.viewExisting}</Button>}
        {formError?.code==='conflict'&&editor.record&&<Button disabled={busy} onClick={()=>readExisting(editor.record!.id)}>{t.reloadConflict}</Button>}
        <footer className={s.actions}><Button disabled={busy} onClick={close}>{t.cancel}</Button><Button variant="primary" type="submit" loading={busy}>{t.save}</Button></footer>
      </form>}
    </Dialog>
    <Dialog open={deleting!==null} onOpenChange={v=>{if(!v)close();}} title={t.removeTitle} description={t.removeDescription} className={s.modal}>
      <div className={s.form}><h2>{t.removeTitle}</h2><p>{deleting?.original}</p><p className={s.meta}>{t.removeDescription}</p>{formError&&<InlineMessage status="error">{t.errors[formError.code]}</InlineMessage>}<footer className={s.actions}><Button disabled={busy} onClick={close}>{t.cancel}</Button><Button variant="primary" loading={busy} onClick={remove}>{t.remove}</Button></footer></div>
    </Dialog>
  </section>;
}

