import {useCallback, useId, useLayoutEffect, useRef, useState, type ReactNode, type ButtonHTMLAttributes} from 'react';
import * as SelectPrimitive from '@radix-ui/react-select';
import * as SwitchPrimitive from '@radix-ui/react-switch';
import {useMotion} from './useMotion';
import s from './ui.module.css';
import {body,runSelectionSpring,settle,type SpringBody} from './selection-motion';

/** Highlight geometry follows actual rows, including wrapping and viewport scrolling. */
export function useSelectionHighlight() {
  const {ref:motionRef,available,active}=useMotion<HTMLDivElement>();
  const [root,setRoot]=useState<HTMLDivElement|null>(null);
  const ref=useCallback((node:HTMLDivElement|null)=>{motionRef.current=node;setRoot(node);},[motionRef]);
  useLayoutEffect(() => {
    const layer = root?.querySelector<HTMLElement>('[data-selection-highlight]');
    if (!root || !layer || !available) return;
    let frame = 0;
    let hovered: HTMLElement | null = null;
    let initialized = false;
    const rowSelector = '[role="option"]';
    const paint = () => {
      frame = 0;
      const focused = document.activeElement instanceof HTMLElement && root.contains(document.activeElement) ? document.activeElement.closest<HTMLElement>(rowSelector) : null;
      const row = hovered?.isConnected ? hovered : focused ?? root.querySelector<HTMLElement>('[data-highlighted], [aria-selected="true"], [data-state="checked"]');
      layer.style.opacity = row ? '1' : '0';
      if (!row) return;
      // Layout offsets are unaffected by the menu's reveal transform.
      let x=0,y=0,node:HTMLElement|null=row;
      while(node&&node!==root){x+=node.offsetLeft;y+=node.offsetTop;node=node.offsetParent as HTMLElement|null;}
      // Intermediate viewports scroll independently; root scroll also moves the layer.
      for(let parent=row.parentElement;parent&&parent!==root;parent=parent.parentElement){x-=parent.scrollLeft;y-=parent.scrollTop;}
      const target={width:row.offsetWidth,height:row.offsetHeight};
      layer.style.transitionDuration = initialized && active ? 'var(--duration-select-glide)' : '0ms';
      layer.style.width = `${target.width}px`;
      layer.style.height = `${target.height}px`;
      layer.style.transform = `translate(${x}px, ${y}px)`;
      initialized = true;
    };
    const schedule = () => { if (!frame) frame = requestAnimationFrame(paint); };
    const pointer = (event: Event) => { const row=event.target instanceof HTMLElement?event.target.closest<HTMLElement>(rowSelector):null;if(row)hovered=row;schedule(); };
    const focus = () => { hovered = null; schedule(); };
    const leave = () => { schedule(); }; // Remember the last browsed row until keyboard/focus takes over.
    const resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(schedule);
    const observeRows=()=>{resize?.disconnect();resize?.observe(root);root.querySelectorAll(rowSelector).forEach(row=>resize?.observe(row));};
    const observer = new MutationObserver(changes=>{if(changes.some(change=>change.type==='childList'))observeRows();schedule();});
    observer.observe(root, {subtree: true, childList: true, characterData:true, attributes: true, attributeFilter: ['data-highlighted', 'data-state', 'aria-selected']});
    observeRows();
    root.addEventListener('pointermove', pointer);
    root.addEventListener('pointerleave', leave);
    root.addEventListener('focusin', focus);
    root.addEventListener('keydown', focus);
    root.addEventListener('scroll', schedule, true);
    window.addEventListener('resize', schedule);
    paint();
    return () => { cancelAnimationFrame(frame); observer.disconnect(); resize?.disconnect(); root.removeEventListener('pointermove', pointer); root.removeEventListener('pointerleave', leave); root.removeEventListener('focusin', focus); root.removeEventListener('keydown', focus); root.removeEventListener('scroll', schedule, true); window.removeEventListener('resize', schedule); };
  }, [available, active, root]);
  return {ref,available,active};
}
export function SelectionHighlight() { return <span className={s.selectionHighlight} data-selection-highlight="" aria-hidden="true"/>; }
/** Searchable model dialogs share ordinary select geometry without changing dialog semantics. */
export function SelectionTrigger({children,className='',...props}:ButtonHTMLAttributes<HTMLButtonElement>){
  return <button type="button" {...props} className={`${s.select} ${className}`} data-state={props['aria-expanded']?'open':'closed'}><span>{children}</span><span className={s.selectChevron} aria-hidden="true"><svg width="16" height="16" viewBox="0 0 16 16" fill="none"><path d="m4 6 4 4 4-4" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round"/></svg></span></button>;
}

function SelectMenu({children}: {children: ReactNode}) {
  const {ref, active} = useSelectionHighlight();
  return <SelectPrimitive.Content ref={ref} className={s.selectContent} data-motion={active ? 'active' : 'static'} position="popper" sideOffset={6} collisionPadding={8}>
    <SelectionHighlight/><SelectPrimitive.Viewport>{children}</SelectPrimitive.Viewport>
  </SelectPrimitive.Content>;
}
export function Select({value, onValueChange, options, label, id, disabled, ...accessibility}: {value: string; onValueChange: (value: string) => void; options: {value: string; label: string}[]; label: string; id?: string; disabled?: boolean; 'aria-describedby'?: string; 'aria-invalid'?: boolean}) {
  return <SelectPrimitive.Root value={value} onValueChange={onValueChange} disabled={disabled}>
    <SelectPrimitive.Trigger {...accessibility} className={s.select} aria-label={label} id={id}><SelectPrimitive.Value/><SelectPrimitive.Icon className={s.selectChevron}><svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="m4 6 4 4 4-4" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round"/></svg></SelectPrimitive.Icon></SelectPrimitive.Trigger>
    <SelectPrimitive.Portal><SelectMenu>{options.map(o => <SelectPrimitive.Item key={o.value} value={o.value} className={s.selectItem}><SelectPrimitive.ItemText>{o.label}</SelectPrimitive.ItemText><SelectPrimitive.ItemIndicator className={s.selectionMark}>✓</SelectPrimitive.ItemIndicator></SelectPrimitive.Item>)}</SelectMenu></SelectPrimitive.Portal>
  </SelectPrimitive.Root>;
}

export function Switch({checked, onCheckedChange, label, id, disabled = false}: {checked: boolean; onCheckedChange: (value: boolean) => void; label: string; id?:string; disabled?: boolean}) {
  const {ref, active} = useMotion<HTMLSpanElement>();
  const thumb=useRef<HTMLSpanElement>(null),spring=useRef(body([checked?20:0]));
  const pending=useRef<{value:boolean}|null>(null);
  useLayoutEffect(()=>{
    const request=pending.current;pending.current=null;
    const node=thumb.current;if(!node)return;
    const state=spring.current;state.target=[checked?20:0];
    const paint=()=>{const stretch=1+Math.min(.65,Math.abs(state.velocity[0]!)/20*.06);node.style.transform=`translateX(${state.position[0]}px) scale(${stretch},${1/stretch})`;};
    if(!active||disabled||request?.value!==checked){settle(state);paint();return;}
    return runSelectionSpring([{body:state,paint,stiffness:360}]);
  },[checked,active,disabled]);
  return <span ref={ref} className={s.switchHit}><SwitchPrimitive.Root id={id} disabled={disabled} className={s.switch} checked={checked} onCheckedChange={value=>{const request={value};pending.current=request;onCheckedChange(value);queueMicrotask(()=>{if(pending.current===request)pending.current=null;});}} aria-label={label}><SwitchPrimitive.Thumb ref={thumb} className={s.thumb}/></SwitchPrimitive.Root></span>;
}

export function SingleChoice({value, onValueChange, options, label, disabled = false}: {value: string; onValueChange: (value: string) => void; options: {value: string; label: string}[]; label: string; disabled?: boolean}) {
  const name = useId();
  const {ref,mark}=useChoiceFeedback(value,disabled);
  return <div ref={ref} role="radiogroup" aria-label={label} className={`${s.singleChoice} ${s.choiceGroup}`}>{options.map(option => <label key={option.value} className={s.choice} data-choice-item="" data-current={value===option.value}>
    <input type="radio" name={name} value={option.value} checked={value === option.value} disabled={disabled} onChange={() => { mark(option.value,()=>onValueChange(option.value)); }}/><span data-choice-face="" data-current={value===option.value} className={s.choiceFace}/><span className={s.choiceLabel}>{option.label}</span>
  </label>)}</div>;
}

export function useChoiceFeedback(value:string,disabled=false){
  const {ref,active}=useMotion<HTMLDivElement>();
  const pending=useRef<{value:string}|null>(null);
  const springs=useRef(new Map<HTMLElement,SpringBody>());
  useLayoutEffect(()=>{
    const request=pending.current;pending.current=null;
    const nodes=Array.from(ref.current?.querySelectorAll<HTMLElement>('[data-choice-item]')??[]);
    const index=nodes.findIndex(node=>node.dataset.current==='true');
    if(index<0)return;
    const selectedWidth=nodes[index]!.offsetWidth;
    for(const node of springs.current.keys())if(!nodes.includes(node))springs.current.delete(node);
    const items=nodes.map((node,i)=>{
      let state=springs.current.get(node);if(!state){state=body([0,1,1]);springs.current.set(node,state);}
      const selected=i===index,distance=Math.abs(i-index);
      const x=active&&!selected?Math.sign(i-index)*(selectedWidth*.1+6):0;
      const size=active?(selected?1.2:.95):1;
      state.target=[x,size,size];state.delay=selected?0:Math.min(3,distance)*22;
      const target=state;
      const paint=()=>{node.style.transform=`translateX(${target.position[0]}px) scale(${target.position[1]},${target.position[2]})`;};
      // The horizontal response leads the vertical swell, producing the jelly split.
      if(active&&!disabled&&request?.value===value&&selected)state.velocity[1]=state.velocity[1]!+3;
      else if(!active||disabled||request?.value!==value)settle(state);
      paint();return {body:state,paint,stiffness:selected?580:580/(1+distance*.12)};
    });
    if(!active||disabled||request?.value!==value)return;
    return runSelectionSpring(items);
  },[value,active,disabled,ref]);
  const mark=(value:string,change:()=>void)=>{const request={value};pending.current=request;change();queueMicrotask(()=>{if(pending.current===request)pending.current=null;});};
  return {ref,mark};
}
