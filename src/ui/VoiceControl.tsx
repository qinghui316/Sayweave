import { useLayoutEffect, useRef } from 'react';
import { AnimatedIcon } from './AnimatedIcon';
import { useMotion } from './useMotion';
import { zh } from '../locales/zh-CN';
import type { Phase } from '../contracts/runtime';
import s from './voice.module.css';
import {MeterWave} from './MeterWave';
import type {SpeechMeterSource} from '../contracts/speech';
const heights = [10,18,12,26,32,16,24,36,18,30,22,14,28,18,8];
export function VoiceControl({ phase, onStart, onCancel, onFinish, motionEnabled = true, level, meterSource, cancelLabel, feedbackLabel, startDisabled = false, restartAvailable = false, compact = false, mode = 'demo' }: { phase: Phase; onStart: () => void; onCancel: () => void; onFinish: () => void; motionEnabled?: boolean; level?: number; meterSource?:SpeechMeterSource; cancelLabel?: string; feedbackLabel?: string; startDisabled?: boolean; restartAvailable?: boolean; compact?: boolean; mode?: 'demo' | 'speech' }) {
  const { ref, active } = useMotion<HTMLDivElement>(motionEnabled);
  const feedback = useRef<HTMLSpanElement>(null);
  const previous = useRef(phase);
  useLayoutEffect(() => {
    const changed = previous.current !== phase;
    previous.current = phase;
    if (changed && phase === 'completed' && active) {
      const animation = feedback.current?.animate?.([{ scale: '1' }, { scale: '1.06' }, { scale: '1' }], { duration: 180, easing: 'ease-out' });
      return () => animation?.cancel();
    }
  }, [phase, active]);
  const recording = phase === 'recording';
  const working = phase === 'connecting' || phase === 'processing';
  const complete = phase === 'completed';
  const failed = phase === 'failed';
  const live = mode === 'speech';
  const title = feedbackLabel ?? (live ? zh.voice.liveStatuses : zh.voice.statuses)[phase];
  return <div ref={ref} className={`${s.control} ${s[phase]} ${compact ? s.compact : ''}`} role="group" aria-label={title} data-phase={phase} data-motion={active ? 'active' : 'static'}>
    {(phase === 'idle' || phase === 'cancelled') && <button disabled={startDisabled} className={s.round} aria-label={live?'开始录音':zh.voice.start} onClick={onStart}><AnimatedIcon icon="mic" size={compact?18:20} tone="voice" enabled={active}/></button>}
    {working && <button className={s.cancelSmall} aria-label={cancelLabel??(live?'取消并丢弃':zh.voice.cancel)} onClick={onCancel}><AnimatedIcon icon="cancel" size={compact?18:16} tone="voice" enabled={active}/></button>}
    {recording && <><button className={s.cancel} aria-label={cancelLabel??(live?'取消并丢弃':zh.voice.cancel)} onClick={onCancel}><AnimatedIcon icon="cancel" size={compact?18:22} tone="voice" enabled={active}/></button>{meterSource||level!==undefined?<MeterWave source={meterSource} level={level} enabled={motionEnabled} compact={compact}/>:<div className={s.wave} aria-hidden="true">{heights.map((height,i)=><i key={i} style={{height:compact?height*2/3:height,animationDelay:`${i*50}ms`}}/>)}</div>}<button className={s.finish} aria-label={live?'结束录音':zh.voice.finish} onClick={onFinish}><AnimatedIcon icon="check" size={compact?18:22} tone="onSignal" enabled={active}/></button></>}
    {/* One persistent SVG preserves processing to completion/error continuity. */}
    <span ref={feedback} className={`${s.statusGlyph} ${complete ? s.done : failed ? s.failure : s.flow}`} data-status-glyph="" aria-hidden="true" style={{ visibility: working || complete || failed ? 'visible' : 'hidden' }}>
      <AnimatedIcon icon={complete ? 'check' : failed ? 'error' : 'pending'} size={compact?18:20} tone={working ? 'signal' : complete ? 'onSignal' : 'onVoiceButton'} enabled={active && (working || complete || failed)} spinning={working}/>
    </span>
    {complete && restartAvailable && <button disabled={startDisabled} className={s.round} style={{position: 'absolute'}} aria-label="开始录音" onClick={onStart}/> }
    {failed && <><span className={s.failureText}>{title}</span><button disabled={startDisabled} className={s.retry} aria-label={live?'重新开始录音':zh.voice.retry} onClick={onStart}><AnimatedIcon icon="retry" size={20} tone="onVoiceButton" enabled={active}/></button></>}
    {!feedbackLabel&&<span className={s.srOnly} role="status">{title}</span>}
  </div>;
}
