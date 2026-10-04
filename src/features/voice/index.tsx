import type { Phase } from '../../contracts/runtime';
import { VoiceControl } from '../../ui';
import s from './voice.module.css';
export function VoiceSurface({ phase, start, finish, cancel, motionEnabled = true,level,error,retry=false }: { phase: Phase; start: () => void; finish: () => void; cancel: () => void; motionEnabled?: boolean;level?:number;error?:string;retry?:boolean }) {
  return <main className={s.surface} title={error}><VoiceControl cancelLabel={retry?'取消本次重识别':undefined} level={level} motionEnabled={motionEnabled} phase={phase} onStart={start} onFinish={finish} onCancel={cancel}/></main>;
}
