import {useState,type ReactNode} from 'react';
import {ActionMenu,LoadingIndicator} from '../../ui';
import {useMotion} from '../../ui/useMotion';
import s from './models.module.css';

interface ModelCardProps {
 name:string;
 subtitle:string;
 selected:boolean;
 disabled?:boolean;
 pending?:boolean;
 status?:string;
 progress?:{value:number;max:number};
 action?:ReactNode;
 onSelect:()=>void;
 items:{label:string;disabled?:boolean;onSelect:()=>void}[];
}

export function ModelCard({name,subtitle,selected,disabled=false,pending=false,status,progress,action,onSelect,items}:ModelCardProps){
 const [menuOpen,setMenuOpen]=useState(false);
 const {ref:card,active}=useMotion<HTMLElement>(),[portalContainer,setPortalContainer]=useState<HTMLElement>();
 return <article ref={card} className={s.modelCard} data-model-card="" data-selected={selected} data-pending={pending} data-motion={active?'active':'static'} data-menu-open={menuOpen} data-action={Boolean(action)}>
  <button type="button" className={s.cardSelect} aria-label={name} aria-pressed={selected} aria-busy={pending||undefined} disabled={disabled} onClick={onSelect}>
   <span className={s.cardMark} aria-hidden="true">{pending&&<LoadingIndicator/>}</span>
   <span className={s.cardCopy}><strong title={name}>{name}</strong><span title={subtitle}>{subtitle}</span></span>
   {status&&<span className={s.cardStatus}>{status}</span>}
   <span className={s.cardSelectedCopy} aria-hidden="true"><span className={s.cardMark}><svg className={s.cardCheck} width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="m5 12 4 4 10-10"/></svg></span><span className={s.cardCopy}><strong data-label={name}/><span data-label={subtitle}/></span>{status&&<span className={s.cardStatus} data-label={status}/>}</span>
  </button>
  <div className={s.cardTools}>{action}<span className={s.cardMore}><ActionMenu label={'更多操作 '+name} items={items} portalContainer={portalContainer} onOpenChange={open=>{if(open)setPortalContainer(card.current?.closest<HTMLElement>('[role="dialog"]')??undefined);setMenuOpen(open);}}/></span></div>
  {progress&&<progress className={s.cardProgress} aria-label={name+'下载进度'} value={progress.value} max={progress.max}/>}
 </article>;
}
