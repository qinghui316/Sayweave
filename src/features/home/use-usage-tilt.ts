import {useEffect,useRef} from 'react';

/** The stationary hit area owns pointer geometry; only the foreground moves. */
export function useUsageTilt(enabled:boolean){
 const area=useRef<HTMLElement>(null),card=useRef<HTMLDivElement>(null);
 useEffect(()=>{
  const target=area.current,face=card.current;
  if(!target||!face||typeof window.matchMedia!=='function')return;
  const reduced=matchMedia('(prefers-reduced-motion: reduce)'),hover=matchMedia('(hover: hover) and (pointer: fine)');
  let frame=0,x=0,y=0;
  const reset=()=>{cancelAnimationFrame(frame);frame=0;face.removeAttribute('data-tilting');face.style.removeProperty('--tilt-x');face.style.removeProperty('--tilt-y');};
  const update=(event:PointerEvent)=>{
   if(!enabled||reduced.matches||!hover.matches||document.hidden||event.pointerType!=='mouse')return;
   const bounds=target.getBoundingClientRect();
   if(!bounds.width||!bounds.height)return;
   x=Math.max(-1,Math.min(1,(event.clientX-bounds.left)/bounds.width*2-1))*3;
   y=-Math.max(-1,Math.min(1,(event.clientY-bounds.top)/bounds.height*2-1))*3;
   if(!frame)frame=requestAnimationFrame(()=>{frame=0;face.dataset.tilting='true';face.style.setProperty('--tilt-x',`${y}deg`);face.style.setProperty('--tilt-y',`${x}deg`);});
  };
  target.addEventListener('pointermove',update);
  target.addEventListener('pointerleave',reset);
  target.addEventListener('pointercancel',reset);
  document.addEventListener('visibilitychange',reset);
  reduced.addEventListener('change',reset);hover.addEventListener('change',reset);
  return()=>{reset();target.removeEventListener('pointermove',update);target.removeEventListener('pointerleave',reset);target.removeEventListener('pointercancel',reset);document.removeEventListener('visibilitychange',reset);reduced.removeEventListener('change',reset);hover.removeEventListener('change',reset);};
 },[enabled]);
 return {area,card};
}
