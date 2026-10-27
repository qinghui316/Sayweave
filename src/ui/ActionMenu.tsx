import {createElement,forwardRef,useCallback,useEffect,useRef,useState} from 'react';
import * as Menu from '@radix-ui/react-dropdown-menu';
import {Ellipsis,Play,Pause,ChevronDown,X,RotateCw,Search,Leaf,Sparkles} from 'lucide';
import {IconButton} from './controls';
import {SelectionHighlight,useSelectionHighlight} from './selection';
import s from './ui.module.css';
const shapes={more:Ellipsis,play:Play,pause:Pause,expand:ChevronDown,close:X,retry:RotateCw,search:Search,manual:Leaf,automatic:Sparkles};
export function ActionIcon({name,size=18}:{name:keyof typeof shapes;size?:number}) {
 return <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{shapes[name].map(([tag,attrs],i)=>createElement(tag,{...attrs,key:i}))}</svg>;
}
export function ActionMenu({label='更多操作',items,onOpenChange,portalContainer}:{label?:string;items:{label:string;disabled?:boolean;onSelect:()=>void}[];onOpenChange?:(open:boolean)=>void;portalContainer?:HTMLElement}) {
 const [view,setView]=useState({open:false,generation:0}),[closingAction,setClosingAction]=useState(false);
 const pending=useRef<{instance:symbol;generation:number;action:()=>void}|null>(null),mounted=useRef(true),generation=useRef(0),opened=useRef(false);
 useEffect(()=>{mounted.current=true;return()=>{mounted.current=false;pending.current=null;};},[]);
 const change=(next:boolean)=>{if(next&&pending.current||next===opened.current)return;opened.current=next;if(next)generation.current++;setView({open:next,generation:generation.current});onOpenChange?.(next);};
 const select=(instance:symbol,epoch:number,action:()=>void)=>{if(!mounted.current||!opened.current||epoch!==generation.current||pending.current)return;pending.current={instance,generation:epoch,action};setClosingAction(true);};
 const settle=(instance:symbol,epoch:number,event:Event)=>{
  if(opened.current||epoch!==generation.current){event.preventDefault();return;}
  const ticket=pending.current;if(!mounted.current||!ticket)return;
  if(ticket.instance!==instance||ticket.generation!==epoch){event.preventDefault();return;}
  pending.current=null;setClosingAction(false);event.preventDefault();ticket.action();
 };
 return <Menu.Root modal={false} open={view.open} onOpenChange={change}><Menu.Trigger asChild><IconButton label={label} disabled={closingAction}><ActionIcon name="more"/></IconButton></Menu.Trigger><Menu.Portal container={portalContainer}><ActionMenuContent key={view.generation} generation={view.generation} items={items} portalContainer={portalContainer} onSelect={select} onClosed={settle}/></Menu.Portal></Menu.Root>;
}
const ActionMenuContent=forwardRef<HTMLDivElement,{items:{label:string;disabled?:boolean;onSelect:()=>void}[];portalContainer?:HTMLElement;generation:number;onSelect:(instance:symbol,generation:number,action:()=>void)=>void;onClosed:(instance:symbol,generation:number,event:Event)=>void}>(function ActionMenuContent({items,portalContainer,generation,onSelect,onClosed},forwardedRef){
 const {ref,active}=useSelectionHighlight('[role="menuitem"]:not([data-disabled])');
 const instance=useRef(Symbol('action-menu'));
 const contentRef=useCallback((node:HTMLDivElement|null)=>{ref(node);if(typeof forwardedRef==='function')forwardedRef(node);else if(forwardedRef)forwardedRef.current=node;},[ref,forwardedRef]);
 return <Menu.Content ref={contentRef} className={s.actionMenu} data-motion={active?'active':'static'} onCloseAutoFocus={event=>onClosed(instance.current,generation,event)} align="end" sideOffset={6} collisionPadding={12} collisionBoundary={portalContainer}><SelectionHighlight/>{items.map(item=><Menu.Item key={item.label} className={s.menuItem} disabled={item.disabled} onSelect={()=>onSelect(instance.current,generation,item.onSelect)}>{item.label}</Menu.Item>)}</Menu.Content>;
});
