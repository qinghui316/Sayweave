import {useCallback,useLayoutEffect,useRef,useState} from 'react';
import {useMotion} from './useMotion';
import {body,runSelectionSpring,settle} from './selection-motion';

/** Only the indicator moves. Accepted business selection owns the destination. */
export function useRubberSelection<T extends HTMLElement=HTMLDivElement>(value:string,disabled=false,options?:{selector?:string;clipRows?:string}){
 const selector=options?.selector??'[aria-current="page"],[data-choice-item][data-current="true"]',clipRows=options?.clipRows;
 const {ref:motionRef,active}=useMotion<T>();
 const [root,setRoot]=useState<T|null>(null);
 const spring=useRef(body([0,0,0,0])),initialized=useRef(false);
 const ref=useCallback((node:T|null)=>{if(motionRef.current!==node)initialized.current=false;motionRef.current=node;setRoot(node);},[motionRef]);
 useLayoutEffect(()=>{
  const layer=root?.querySelector<HTMLElement>('[data-rubber-indicator]');
  if(!root||!layer)return;
  let stop:(()=>void)|undefined;
  const rectangle=(row:HTMLElement)=>{
   let x=0,y=0,node:HTMLElement|null=row;
   while(node&&node!==root){x+=node.offsetLeft;y+=node.offsetTop;node=node.offsetParent as HTMLElement|null;}
   return {row,x,y,width:row.offsetWidth,height:row.offsetHeight};
  };
  let trackedRows:ReturnType<typeof rectangle>[]=[],trackedSelected:HTMLElement|null=null;
  const measure=(animate=false)=>{
   const selected=root.querySelector<HTMLElement>(selector);
   const rows=clipRows?Array.from(root.querySelectorAll<HTMLElement>(clipRows),rectangle):[];
   trackedRows=rows;trackedSelected=selected;
   layer.style.opacity=selected?'1':'0';
   if(!selected){stop?.();initialized.current=false;for(const {row} of rows){row.style.setProperty('--selection-clip','inset(0 0 100%)');row.style.setProperty('--card-tools-color','var(--text-primary)');}return;}
   const state=spring.current;
   const target=clipRows?rectangle(selected):{x:selected.offsetLeft,y:selected.offsetTop,width:selected.offsetWidth,height:selected.offsetHeight};
   state.target=[target.x,target.y,target.width,target.height];
   const paint=()=>{
    const [x,y,w,h]=state.position as [number,number,number,number];
    const sx=Math.min(8,Math.abs(state.target[0]!-x)*.12),sy=Math.min(8,Math.abs(state.target[1]!-y)*.12);
    layer.style.width=`${w+sx}px`;layer.style.height=`${h+sy}px`;
    layer.style.transform=`translate(${x-sx/2}px,${y-sy/2}px)`;
    // The same moving plate reveals inverse foreground, never moves or fades labels.
    for(const {row,x:rx,y:ry,width,height} of rows){
     const left=Math.max(0,x-sx/2-rx),top=Math.max(0,y-sy/2-ry),right=Math.max(0,rx+width-x-w-sx/2),bottom=Math.max(0,ry+height-y-h-sy/2);
     row.style.setProperty('--selection-clip',left+right>=width||top+bottom>=height?'inset(0 0 100%)':`inset(${top}px ${right}px ${bottom}px ${left}px)`);
     row.style.setProperty('--card-tools-color',ry+height/2>=y-sy/2&&ry+height/2<=y+h+sy/2?'var(--selected-fg)':'var(--text-primary)');
    }
   };
   stop?.();if(!initialized.current||!animate||!active||disabled){settle(state);paint();}
   else stop=runSelectionSpring([{body:state,paint}]);
   initialized.current=true;
  };
  measure(true);
  let width=root.clientWidth,height=root.clientHeight;
  const resize=typeof ResizeObserver==='undefined'?null:new ResizeObserver(()=>{if(root.clientWidth!==width||root.clientHeight!==height){width=root.clientWidth;height=root.clientHeight;measure();}});resize?.observe(root);
  // Equal-sized row replacement cannot be observed by root size alone. Snap to
  // the current DOM and retire the old paint closure; never observe our styles.
  const children=clipRows?new MutationObserver(()=>{
   const rows=Array.from(root.querySelectorAll<HTMLElement>(clipRows),rectangle);
   if(root.querySelector(selector)!==trackedSelected||rows.length!==trackedRows.length||rows.some((next,i)=>{const before=trackedRows[i]!;return next.row!==before.row||next.x!==before.x||next.y!==before.y||next.width!==before.width||next.height!==before.height;}))measure();
  }):null;
  children?.observe(root,{subtree:true,childList:true});
  return()=>{stop?.();resize?.disconnect();children?.disconnect();};
 },[value,active,disabled,root,selector,clipRows]);
 return {ref,active};
}
export function RubberIndicator(){return <span data-rubber-indicator="" aria-hidden="true"/>;}
