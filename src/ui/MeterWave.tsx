import {useLayoutEffect, useRef} from 'react';
import type {SpeechMeterSource} from '../contracts/speech';
import {useMotion} from './useMotion';
import {WaveEnvelope} from './waveform';
import s from './voice.module.css';

/** High-frequency notifications and painting stay inside this leaf. */
export function MeterWave({source, enabled=true, compact=false, level}: {source?:SpeechMeterSource;enabled?:boolean;compact?:boolean;level?:number}) {
  const {ref,available,reduced}=useMotion<HTMLDivElement>(enabled);
  const envelope=useRef(new WaveEnvelope());
  const fallback=useRef(level);
  useLayoutEffect(()=>{fallback.current=level;},[level]);
  useLayoutEffect(()=>{
    if(!available)return;
    let current=source?.getSnapshot()??null;
    const release=source?.subscribe(()=>{current=source.getSnapshot();});
    let frame=0;
    let last=performance.now();
    const draw=(now:number)=>{
      const meter=current;
      const values=envelope.current.draw(meter,now,Math.min(100,now-last),reduced,fallback.current);
      last=now;
      const bars=ref.current?.children;
      if(bars)Array.from(bars).forEach((bar,i)=>{
        const node=bar as HTMLElement;
        node.style.transform=`scaleY(${values[i]})`;
        node.style.opacity=String(.7+(values[i]??0)*.3);
      });
      frame=requestAnimationFrame(draw);
    };
    draw(last);
    return()=>{cancelAnimationFrame(frame);release?.();};
  },[source,available,reduced,ref]);
  return <div ref={ref} className={s.wave} data-live="true" aria-hidden="true">{Array.from({length:15},(_,i)=><i key={i} style={{height:compact?24:36}}/>)}</div>;
}
