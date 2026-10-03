import { useEffect, useRef, useState } from 'react';
import { AsyncStatusFeedback, Button, CopyFeedbackButton, type AsyncStatus } from '../ui';
import { zh } from '../locales/zh-CN';
import s from './dev.module.css';
export function MotionExamples() {
  const [status, setStatus] = useState<AsyncStatus>('idle');
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const t = zh.motion;
  function connect(result: 'success'|'error') {
    clearTimeout(timer.current);
    setStatus('pending');
    timer.current = setTimeout(() => setStatus(result), 700);
  }
  async function simulateCopy(fail: boolean) { await new Promise(resolve => setTimeout(resolve, 400)); if (fail) throw new Error('demo'); }
  return <section className={s.group} aria-label={t.title}><h2>{t.title}</h2><p>{t.note}</p>
    <div className={s.row}><span>{t.copySuccess}</span><CopyFeedbackButton onCopy={() => simulateCopy(false)}/><span>{t.copyError}</span><CopyFeedbackButton onCopy={() => simulateCopy(true)}/></div>
    <div className={s.row}><Button onClick={() => connect('success')}>{t.connect}</Button><Button onClick={() => connect('error')}>{t.connectError}</Button><Button onClick={() => { clearTimeout(timer.current); setStatus('idle'); }}>{t.reset}</Button></div>
    <AsyncStatusFeedback status={status} message={t.connection[status]}/>
  </section>;
}
