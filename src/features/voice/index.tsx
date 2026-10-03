import type { Phase } from '../../contracts/runtime';
import { VoiceControl } from '../../ui';
import s from './voice.module.css';
export function VoiceSurface({ phase, start, finish, cancel, motionEnabled = true }: { phase: Phase; start: () => void; finish: () => void; cancel: () => void; motionEnabled?: boolean }) {
  return <main className={s.surface}><VoiceControl motionEnabled={motionEnabled} phase={phase} onStart={start} onFinish={finish} onCancel={cancel}/></main>;
}
