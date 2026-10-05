import { useLayoutEffect, useRef, useCallback } from 'react';
import { AnimatedIcon } from './AnimatedIcon';
import { useMotion } from './useMotion';
import { zh } from '../locales/zh-CN';
import type { Phase } from '../contracts/runtime';
import s from './voice.module.css';
import { waveformSample } from './waveform';
const heights = [10,18,12,26,32,16,24,36,18,30,22,14,28,18,8];
export function VoiceControl({ phase, onStart, onCancel, onFinish, motionEnabled = true, level, cancelLabel, feedbackLabel, mode = 'demo' }: { phase: Phase; onStart: () => void; onCancel: () => void; onFinish: () => void; motionEnabled?: boolean; level?: number; cancelLabel?: string; feedbackLabel?: string; mode?: 'demo' | 'speech' }) {
  const { ref, active, available } = useMotion<HTMLDivElement>(motionEnabled);
  const wave = useRef<HTMLDivElement>(null);
  const samples = useRef<number[]>([]);
  const measured = useRef(level);
  const liveLevel = level !== undefined;
  useLayoutEffect(() => { measured.current = level; }, [level]);
  const draw = useCallback(() => {
    const previousLevel = samples.current.at(-1) ?? 0;
    samples.current = [...samples.current, waveformSample(previousLevel, measured.current ?? 0)].slice(-heights.length);
    const bars = wave.current?.children;
    if (bars) Array.from(bars).forEach((bar, i) => {
      const sample = samples.current[Math.max(0, samples.current.length - heights.length + i)] ?? 0;
      (bar as HTMLElement).style.transform = `scaleY(${.12 + sample * .88})`;
    });
  }, []);
  useLayoutEffect(() => {
    if (phase !== 'recording' || !liveLevel || !available) { samples.current = []; return; }
    draw();
    const timer = setInterval(draw, 50);
    return () => { clearInterval(timer); samples.current = []; };
  }, [phase, liveLevel, available, draw]);
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
  return <div ref={ref} className={`${s.control} ${s[phase]}`} role="group" aria-label={title} data-phase={phase} data-motion={active ? 'active' : 'static'}>
    {(phase === 'idle' || phase === 'cancelled') && <button className={s.round} aria-label={live?'开始录音':zh.voice.start} onClick={onStart}><AnimatedIcon icon="mic" size={20} tone="inverse" enabled={active}/></button>}
    {working && <button className={s.cancelSmall} aria-label={cancelLabel??(live?'取消并丢弃':zh.voice.cancel)} onClick={onCancel}><AnimatedIcon icon="cancel" size={16} tone="inverse" enabled={active}/></button>}
    {recording && <><button className={s.cancel} aria-label={cancelLabel??(live?'取消并丢弃':zh.voice.cancel)} onClick={onCancel}><AnimatedIcon icon="cancel" size={22} tone="inverse" enabled={active}/></button><div ref={wave} className={s.wave} data-live={level!==undefined} aria-hidden="true">{heights.map((height,i) => <i key={i} style={{height,animationDelay:`${i*50}ms`}}/>)}</div><button className={s.finish} aria-label={live?'结束录音':zh.voice.finish} onClick={onFinish}><AnimatedIcon icon="check" size={22} enabled={active}/></button></>}
    {/* One persistent SVG preserves processing to completion/error continuity. */}
    <span ref={feedback} className={`${s.statusGlyph} ${complete ? s.done : failed ? s.failure : s.flow}`} data-status-glyph="" aria-hidden="true" style={{ visibility: working || complete || failed ? 'visible' : 'hidden' }}>
      <AnimatedIcon icon={complete ? 'check' : failed ? 'error' : 'pending'} size={20} tone={working ? 'signal' : 'primary'} enabled={active && (working || complete || failed)} spinning={working}/>
    </span>
    {failed && <><span className={s.failureText}>{title}</span><button className={s.retry} aria-label={live?'重新开始录音':zh.voice.retry} onClick={onStart}><AnimatedIcon icon="retry" size={20} enabled={active}/></button></>}
    {!feedbackLabel&&<span className={s.srOnly} role="status">{title}</span>}
  </div>;
}
