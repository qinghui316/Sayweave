import type { CSSProperties } from 'react';
import s from './ui.module.css';
const assets = import.meta.glob<string>('../assets/*.svg', { query: '?url', import: 'default', eager: true });
export type IconName = 'house'|'clock'|'book-open'|'sliders-horizontal'|'settings'|'info'|'minus'|'square'|'x'|'mic'|'languages'|'square-pen';
export function Icon({ name, size = 24, signal = false }: { name: IconName; size?: number; signal?: boolean }) {
  const url = assets[`../assets/${name}.svg`];
  return <span aria-hidden="true" className={`${s.icon} ${signal ? s.signal : ''}`} style={{ '--icon-url': `url("${url}")`, width: size, height: size } as CSSProperties} />;
}
