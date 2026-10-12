import {useCallback,useLayoutEffect,useRef,useState} from 'react';
import {useMotion} from './useMotion';
import {body,runSelectionSpring,settle} from './selection-motion';

/** Only the indicator moves. Accepted business selection owns the destination. */
export function useRubberSelection<T extends HTMLElement=HTMLDivElement>(value:string,disabled=false){
 const {ref:motionRef,active}=useMotion<T>();
 const [root,setRoot]=useState<T|null>(null);
 const spring=useRef(body([0,0,0,0])),initialized=useRef(false);
 const ref=useCallback((node:T|null)=>{if(motionRef.current!==node)initialized.current=false;motionRef.current=node;setRoot(node);},[motionRef]);
 useLayoutEffect(()=>{
  const layer=root?.querySelector<HTMLElement>('[data-rubber-indicator]');
  if(!root||!layer)return;
  let stop:(()=>void)|undefined;
  const measure=(animate=false)=>{
   const selected=root.querySelector<HTMLElement>('[aria-current="page"],[data-choice-item][data-current="true"]');
   layer.style.opacity=selected?'1':'0';if(!selected)return;
   const state=spring.current;
   state.target=[selected.offsetLeft,selected.offsetTop,selected.offsetWidth,selected.offsetHeight];
   const paint=()=>{
    const [x,y,w,h]=state.position as [number,number,number,number];
    const sx=Math.min(8,Math.abs(state.target[0]!-x)*.12),sy=Math.min(8,Math.abs(state.target[1]!-y)*.12);
    layer.style.width=`${w+sx}px`;layer.style.height=`${h+sy}px`;
    layer.style.transform=`translate(${x-sx/2}px,${y-sy/2}px)`;
   };
   stop?.();if(!initialized.current||!animate||!active||disabled){settle(state);paint();}
   else stop=runSelectionSpring([{body:state,paint}]);
   initialized.current=true;
  };
  measure(true);
  let width=root.clientWidth,height=root.clientHeight;
  const resize=typeof ResizeObserver==='undefined'?null:new ResizeObserver(()=>{if(root.clientWidth!==width||root.clientHeight!==height){width=root.clientWidth;height=root.clientHeight;measure();}});resize?.observe(root);
  return()=>{stop?.();resize?.disconnect();};
 },[value,active,disabled,root]);
 return {ref};
}
export function RubberIndicator(){return <span data-rubber-indicator="" aria-hidden="true"/>;}
