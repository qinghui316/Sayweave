import {useEffect,useId,useLayoutEffect,useRef,useState} from 'react';
import {TextInput,IconButton} from './controls';
import {ActionIcon} from './ActionMenu';
import s from './expandable-search.module.css';

/** Presentation only: each page owns its query, pagination and request timing. */
export function ExpandableSearch({value,onValueChange,label,triggerLabel=label,placeholder=label,anchor='right'}:{value:string;onValueChange:(value:string)=>void;label:string;triggerLabel?:string;placeholder?:string;anchor?:'left'|'right'}){
 const [expanded,setExpanded]=useState(false);
 const open=expanded||value.length>0;
 const id=useId(),root=useRef<HTMLDivElement>(null),field=useRef<HTMLInputElement>(null);
 const composing=useRef(false),restoreFocus=useRef(false),focusOnOpen=useRef(false);
 useLayoutEffect(()=>{
  if(open&&focusOnOpen.current){focusOnOpen.current=false;field.current?.focus();}
  if(!open&&restoreFocus.current){restoreFocus.current=false;root.current?.querySelector('button')?.focus();}
 },[open]);
 useEffect(()=>{
  if(!open)return;
  const outside=(event:PointerEvent)=>{if(!value&&!composing.current&&!root.current?.contains(event.target as Node))setExpanded(false);};
  document.addEventListener('pointerdown',outside);
  return()=>document.removeEventListener('pointerdown',outside);
 },[open,value]);
 return <div ref={root} className={s.slot} data-expandable-search="" data-anchor={anchor}>
  {!open?<IconButton label={triggerLabel} aria-expanded={false} aria-controls={id} className={s.trigger} onClick={()=>{focusOnOpen.current=true;setExpanded(true);}}><ActionIcon name="search" size={16}/></IconButton>:
   <div id={id} className={s.box}><div className={s.contents}><ActionIcon name="search" size={16}/><TextInput ref={field} aria-label={label} placeholder={placeholder} value={value} maxLength={256}
    onFocus={()=>setExpanded(true)} onCompositionStart={()=>{composing.current=true;}} onCompositionEnd={()=>{composing.current=false;}}
    onKeyDown={event=>{if(event.key!=='Escape'||event.nativeEvent.isComposing||composing.current||event.keyCode===229)return;event.preventDefault();event.stopPropagation();restoreFocus.current=true;onValueChange('');setExpanded(false);}}
    onChange={event=>onValueChange(event.target.value)}/></div></div>}
 </div>;
}
