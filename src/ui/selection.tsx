import {useCallback, useId, useLayoutEffect, useRef, useState, type ReactNode} from 'react';
import * as SelectPrimitive from '@radix-ui/react-select';
import * as SwitchPrimitive from '@radix-ui/react-switch';
import {useMotion} from './useMotion';
import s from './ui.module.css';

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
      const bounds = root.getBoundingClientRect(), target = row.getBoundingClientRect();
      layer.style.transitionDuration = initialized && active ? 'var(--duration-select-glide)' : '0ms';
      layer.style.width = `${target.width}px`;
      layer.style.height = `${target.height}px`;
      layer.style.transform = `translate(${target.left - bounds.left + root.scrollLeft}px, ${target.top - bounds.top + root.scrollTop}px)`;
      initialized = true;
    };
    const schedule = () => { if (!frame) frame = requestAnimationFrame(paint); };
    const pointer = (event: Event) => { hovered = event.target instanceof HTMLElement ? event.target.closest<HTMLElement>(rowSelector) : null; schedule(); };
    const focus = () => { hovered = null; schedule(); };
    const leave = () => { hovered = null; schedule(); };
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
  const thumb = useRef<HTMLSpanElement>(null);
  const pending = useRef<{value: boolean; from: string} | null>(null);
  useLayoutEffect(() => {
    const request = pending.current; pending.current = null;
    const node = thumb.current;
    if (!node || !active || disabled || request?.value !== checked) return;
    const x = checked ? 20 : 0;
    const animation = node.animate?.([{transform: request.from}, {transform: `translateX(${x}px) scale(1.12, .893)`, offset: .65}, {transform: `translateX(${x}px) scale(1)`}], {duration: 200, easing: 'cubic-bezier(.2,.8,.2,1)'});
    return () => animation?.cancel();
  }, [checked, active, disabled]);
  return <span ref={ref} className={s.switchHit}><SwitchPrimitive.Root id={id} disabled={disabled} className={s.switch} checked={checked} onCheckedChange={value => { const request={value, from: thumb.current ? getComputedStyle(thumb.current).transform : 'none'};pending.current=request;onCheckedChange(value);queueMicrotask(()=>{if(pending.current===request)pending.current=null;}); }} aria-label={label}><SwitchPrimitive.Thumb ref={thumb} className={s.thumb}/></SwitchPrimitive.Root></span>;
}

export function SingleChoice({value, onValueChange, options, label, disabled = false}: {value: string; onValueChange: (value: string) => void; options: {value: string; label: string}[]; label: string; disabled?: boolean}) {
  const name = useId();
  const {ref,mark}=useChoiceFeedback(value,disabled);
  return <div ref={ref} role="radiogroup" aria-label={label} className={s.singleChoice}>{options.map(option => <label key={option.value} className={s.choice}>
    <input type="radio" name={name} value={option.value} checked={value === option.value} disabled={disabled} onChange={() => { mark(option.value,()=>onValueChange(option.value)); }}/><span data-choice-face="" data-current={value===option.value} className={s.choiceFace}/><span className={s.choiceLabel}>{option.label}</span>
  </label>)}</div>;
}

export function useChoiceFeedback(value:string,disabled=false){
  const {ref,active}=useMotion<HTMLDivElement>();
  const pending=useRef<{value:string}|null>(null);
  useLayoutEffect(()=>{
    const request=pending.current;pending.current=null;
    if(!active||disabled||request?.value!==value)return;
    const face=ref.current?.querySelector<HTMLElement>('[data-choice-face][data-current="true"]');
    const animation=face?.animate?.([{transform:'scale(1)'},{transform:'scale(1.06, .96)',offset:.35},{transform:'scale(.99, 1.02)',offset:.75},{transform:'scale(1)'}],{duration:220,easing:'ease-out'});
    return()=>animation?.cancel();
  },[value,active,disabled,ref]);
  const mark=(value:string,change:()=>void)=>{const request={value};pending.current=request;change();queueMicrotask(()=>{if(pending.current===request)pending.current=null;});};
  return {ref,mark};
}
