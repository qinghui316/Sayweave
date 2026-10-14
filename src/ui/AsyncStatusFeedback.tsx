import { AnimatedIcon, type AnimatedIconName } from './AnimatedIcon';
import s from './motion.module.css';
export type AsyncStatus = 'idle'|'pending'|'success'|'error';
const icons: Record<AsyncStatus, AnimatedIconName> = { idle: 'link', pending: 'pending', success: 'check', error: 'error' };
export function AsyncStatusFeedback({ status, message, motionEnabled = true }: { status: AsyncStatus; message: string; motionEnabled?: boolean }) {
  return <div className={s.feedback} data-status={status} role="status" aria-live="polite"><span className={`${s.badge} ${status === 'success' ? s.success : ''}`}><AnimatedIcon icon={icons[status]} tone={status==='success'?'onSignal':'primary'} enabled={motionEnabled} spinning={status === 'pending'}/></span><span>{message}</span></div>;
}
