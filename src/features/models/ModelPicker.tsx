import {useState} from 'react';
import {Button,Dialog,TextInput} from '../../ui';
import {SelectionHighlight,useSelectionHighlight} from '../../ui/selection';
import s from './models.module.css';

export function ModelPicker({id,value,ids,loading,error,onOpen,onRefresh,onChange,...accessibility}:{id:string;value:string;ids:string[];loading:boolean;error:string|null;onOpen:()=>void;onRefresh:()=>void;onChange:(value:string)=>void;'aria-describedby'?:string;'aria-invalid':boolean}) {
 const [open,setOpen]=useState(false),[query,setQuery]=useState(''),[manual,setManual]=useState(false);
 const filtered=ids.filter(item=>item.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
 function select(value:string){onChange(value);setOpen(false);}
 return <>
  <div className={s.modelChoice}>
   {manual?<TextInput {...accessibility} id={id} value={value} placeholder="填写模型 ID" onChange={e=>onChange(e.target.value)}/>:<Button {...accessibility} id={id} className={s.modelTrigger} aria-haspopup="dialog" aria-expanded={open} onClick={()=>{setOpen(true);setQuery('');onOpen();}}>{value||'选择文字模型'}<span aria-hidden="true">⌄</span></Button>}
   <Button variant="quiet" onClick={()=>setManual(v=>!v)}>{manual?'从列表选择':'手动填写'}</Button>
  </div>
  <Dialog open={open} onOpenChange={setOpen} title="选择文字模型" description="搜索服务返回的模型，选择后可主动测试连接。" className={s.modelDialog}>
   <div className={s.modelSearch}><TextInput aria-label="搜索模型" placeholder="搜索模型" value={query} onChange={e=>setQuery(e.target.value)}/><Button loading={loading} onClick={onRefresh}>刷新</Button></div>
   {error&&<p role="alert" className={s.meta}>{error}</p>}
   {loading&&<p role="status" className={s.meta}>正在同步模型…</p>}
   <ModelOptions filtered={filtered} value={value} select={select}/>
   {!loading&&filtered.length===0&&<p className={s.meta}>{ids.length?'没有匹配的模型。':'暂时没有模型列表，可关闭后手动填写模型 ID。'}</p>}
  </Dialog>
 </>;
}

function ModelOptions({filtered,value,select}:{filtered:string[];value:string;select:(value:string)=>void}){
 const {ref}=useSelectionHighlight();
 return <div ref={ref} role="listbox" aria-label="可用文字模型" className={s.modelOptions} onKeyDown={e=>{
    const items=Array.from(e.currentTarget.querySelectorAll<HTMLButtonElement>('button'));
    const index=items.indexOf(document.activeElement as HTMLButtonElement);
    const next=e.key==='ArrowDown'?Math.min(index+1,items.length-1):e.key==='ArrowUp'?Math.max(index-1,0):e.key==='Home'?0:e.key==='End'?items.length-1:-1;
    if(next>=0){e.preventDefault();items[next]?.focus();}
   }}><SelectionHighlight/>{filtered.map(model=><Button key={model} role="option" aria-selected={model===value} onClick={()=>select(model)}>{model}{model===value&&<span aria-hidden="true">✓</span>}</Button>)}</div>;
}
