import {useEffect,useId,useLayoutEffect,useRef,useState,useSyncExternalStore} from 'react';
import {createPortal} from 'react-dom';
import type {UpdatePort,UpdateSnapshot} from '../contracts/update';
import {ActionIcon} from './ActionMenu';
import {Icon} from './Icon';
import {IconButton} from './controls';
import {UpdatePanel} from './UpdatePanel';
import {hasUpdateOffer,updateLabel} from './update-state';
import s from './update-dock.module.css';

type Props={port:UpdatePort;onAbout:()=>void;onInstall:()=>void;preparing:boolean;error:string|null;dismissKey:string};
export function UpdateDock(props:Props) {
  const value=useSyncExternalStore(props.port.subscribe,props.port.snapshot),offer=hasUpdateOffer(value);
  return <UpdateDockContent key={`${props.dismissKey}:${offer}`} {...props} value={value} offer={offer}/>;
}
function UpdateDockContent({port,onAbout,onInstall,preparing,error,value,offer}:Props&{value:UpdateSnapshot;offer:boolean}) {
  const id=useId();
  const [open,setOpen]=useState(false),[position,setPosition]=useState<{left:number;top:number}|null>(null);
  const trigger=useRef<HTMLButtonElement>(null),panel=useRef<HTMLElement>(null),focusOnOpen=useRef(false);
  useLayoutEffect(()=>{
    if(!open||!offer)return;
    const measure=()=>{
      if(!trigger.current||!panel.current)return;
      const anchor=trigger.current.getBoundingClientRect(),box=panel.current.getBoundingClientRect();
      const left=Math.max(12,Math.min(anchor.right-box.width,window.innerWidth-box.width-12));
      const candidate=anchor.top-box.height-12;
      const top=Math.max(12,Math.min(candidate>=12?candidate:anchor.bottom+12,window.innerHeight-box.height-12));
      setPosition({left,top});
    };
    measure();
    if(focusOnOpen.current){focusOnOpen.current=false;panel.current?.focus();}
    const observer=typeof ResizeObserver==='undefined'?null:new ResizeObserver(measure);
    if(panel.current)observer?.observe(panel.current);
    window.addEventListener('resize',measure);window.addEventListener('scroll',measure,true);
    return()=>{observer?.disconnect();window.removeEventListener('resize',measure);window.removeEventListener('scroll',measure,true);};
  },[open,offer]);
  useEffect(()=>{
    if(!open)return;
    const outside=(event:PointerEvent)=>{const target=event.target as Node;if(!panel.current?.contains(target)&&!trigger.current?.contains(target))setOpen(false);};
    const escape=(event:KeyboardEvent)=>{if(event.key==='Escape'){event.preventDefault();setOpen(false);trigger.current?.focus();}};
    document.addEventListener('pointerdown',outside,true);document.addEventListener('keydown',escape);
    return()=>{document.removeEventListener('pointerdown',outside,true);document.removeEventListener('keydown',escape);};
  },[open]);
  const pending=['checking','verifying','preparing','stopping','installing'].includes(value.state);
  const label=offer?updateLabel(value.state,value.version,value.progress):'关于';
  return <>
    <button ref={trigger} type="button" className={s.trigger} aria-label={label} title={label} aria-haspopup={offer?'dialog':undefined} aria-expanded={offer?open:undefined} aria-controls={offer&&open?id:undefined} onClick={()=>{if(!offer){onAbout();return;}focusOnOpen.current=!open;setOpen(v=>!v);}}>
      {!offer?<Icon name="info" size={18}/>:value.state==='failed'||value.recovery?<span className={s.failed} aria-hidden="true">!</span>:value.state==='readyToInstall'?<ActionIcon name="retry" size={18}/>:<svg viewBox="0 0 40 40" aria-hidden="true" className={pending?s.spin:undefined}><circle className={s.track} cx="20" cy="20" r="15"/><circle className={s.progress} cx="20" cy="20" r="15" pathLength="100" strokeDasharray="100" strokeDashoffset={pending?75:100-(value.progress??0)}/></svg>}
    </button>
    {offer&&open&&createPortal(<section ref={panel} id={id} role="dialog" aria-label="软件更新" tabIndex={-1} className={s.popover} style={position??{visibility:'hidden'}}>
      <div className={s.header}><span>{value.version?`新版 ${value.version}`:'更新恢复'}</span><IconButton label="关闭更新详情" onClick={()=>{setOpen(false);trigger.current?.focus();}}><ActionIcon name="close" size={16}/></IconButton></div>
      <UpdatePanel port={port} onInstall={onInstall} preparing={preparing} error={error} showIdentity={false}/>
    </section>,document.body)}
  </>;
}
