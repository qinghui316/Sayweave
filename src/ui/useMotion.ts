import { createContext, createElement, useContext, useEffect, useRef, useState, type ReactNode } from 'react';

const MotionAvailability = createContext(true);
/** Presentation-only scope; the app supplies visibility, never business phase. */
export function MotionScope({ enabled, children }: { enabled: boolean; children: ReactNode }) {
  const parentEnabled = useContext(MotionAvailability);
  return createElement(MotionAvailability.Provider, { value: parentEnabled && enabled }, children);
}

/** Native visibility supplements document visibility for hidden WebViews. */
export function useMotion<T extends HTMLElement = HTMLSpanElement>(enabled = true) {
  const scopeEnabled = useContext(MotionAvailability);
  const ref = useRef<T>(null);
  const [onScreen, setOnScreen] = useState(true);
  const [visible, setVisible] = useState(() => !document.hidden);
  const [reduced, setReduced] = useState(() => window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false);
  useEffect(() => {
    const media = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    const preference = () => setReduced(media?.matches ?? false);
    const visibility = () => setVisible(!document.hidden);
    media?.addEventListener('change', preference);
    document.addEventListener('visibilitychange', visibility);
    const observer = typeof IntersectionObserver === 'undefined' ? undefined : new IntersectionObserver(entries => setOnScreen(entries[0]?.isIntersecting ?? false));
    if (ref.current) observer?.observe(ref.current);
    return () => {
      observer?.disconnect();
      media?.removeEventListener('change', preference);
      document.removeEventListener('visibilitychange', visibility);
    };
  }, []);
  const available = scopeEnabled && enabled && visible && onScreen;
  return { ref, active: available && !reduced, available, reduced };
}
