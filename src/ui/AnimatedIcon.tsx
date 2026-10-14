import { useLayoutEffect, useRef } from 'react';
import { MorphIcon, type MorphHandle, type IconInput } from 'morphicons/react';
import { svgToIcon } from 'morphicons/adapters';
import { Check, Eye, EyeOff, Copy, LoaderCircle, Link, CircleAlert, RotateCw } from 'lucide';
import mic from '../assets/mic.svg?raw';
import cancel from '../assets/x.svg?raw';
import { useMotion } from './useMotion';
import s from './motion.module.css';

export const motionIcons = { mic: svgToIcon(mic), cancel: svgToIcon(cancel), check: Check, eye: Eye, eyeOff: EyeOff, copy: Copy, pending: LoaderCircle, link: Link, error: CircleAlert, retry: RotateCw } satisfies Record<string, IconInput>;
export type AnimatedIconName = keyof typeof motionIcons;
export function AnimatedIcon({ icon, size = 20, tone = 'primary', enabled = true, spinning = false }: { icon: AnimatedIconName; size?: number; tone?: 'primary'|'inverse'|'signal'|'onSignal'|'voice'|'onVoiceButton'; enabled?: boolean; spinning?: boolean }) {
  const { ref, active, available, reduced } = useMotion(enabled);
  const driver = useRef<MorphHandle>(null);
  const previous = useRef(icon);
  // Stop in-flight geometry immediately when hidden/reduced; restoration never replays it.
  useLayoutEffect(() => {
    const changed = previous.current !== icon;
    previous.current = icon;
    if (!active) driver.current?.set(motionIcons[icon]);
    if (changed && available && reduced) {
      const fade = ref.current?.animate?.([{ opacity: .8 }, { opacity: 1 }], { duration: 120 });
      return () => fade?.cancel();
    }
  }, [active, available, reduced, icon, ref]);
  return <span ref={ref} className={s.icon} data-motion={active ? 'active' : 'static'} data-icon={icon} style={{ width: size, height: size }} aria-hidden="true">
    <MorphIcon ref={driver} icon={motionIcons[icon]} size={size} color={`var(--${tone === 'voice' ? 'voice-fg' : tone === 'onVoiceButton' ? 'voice-button-fg' : tone === 'onSignal' ? 'on-signal' : tone === 'signal' ? 'signal' : tone === 'inverse' ? 'text-inverse' : 'text-primary'})`} strokeWidth={2} absoluteStrokeWidth spring="snappy" reducedMotion={active ? 'user' : 'always'} className={spinning && active ? s.spin : undefined}/>
  </span>;
}
