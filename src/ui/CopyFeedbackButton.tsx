import { useEffect, useRef, useState } from 'react';
import { AnimatedIcon, type AnimatedIconName } from './AnimatedIcon';
import { Button } from './controls';
import { zh } from '../locales/zh-CN';
import type { AsyncStatus } from './AsyncStatusFeedback';
import s from './motion.module.css';
const icons: Record<AsyncStatus, AnimatedIconName> = { idle: 'copy', pending: 'pending', success: 'check', error: 'error' };
export function CopyFeedbackButton({ onCopy, disabled = false, motionEnabled = true }: { onCopy: () => Promise<void>; disabled?: boolean; motionEnabled?: boolean }) {
  const [status, setStatus] = useState<AsyncStatus>('idle');
  const generation = useRef({ value: 0 });
  const alive = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => {
    const requests = generation.current;
    alive.current = true;
    return () => { alive.current = false; requests.value++; clearTimeout(timer.current); };
  }, []);
  async function copy() {
    const request = ++generation.current.value;
    clearTimeout(timer.current);
    setStatus('pending');
    try {
      await onCopy();
      if (!alive.current || generation.current.value !== request) return;
      setStatus('success');
      timer.current = setTimeout(() => { if (alive.current && generation.current.value === request) setStatus('idle'); }, 1500);
    } catch {
      if (alive.current && generation.current.value === request) setStatus('error');
    }
  }
  return <Button disabled={disabled} aria-label={zh.motion.copy[status]} aria-busy={status === 'pending' || undefined} onClick={() => void copy()} className={s.copy}>
    <AnimatedIcon icon={icons[status]} tone={status === 'success' ? 'signal' : 'primary'} spinning={status === 'pending'} enabled={motionEnabled}/><span role="status" aria-live="polite">{zh.motion.copy[status]}</span>
  </Button>;
}
