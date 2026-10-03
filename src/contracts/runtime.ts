export type Surface = 'main' | 'overlay';
export type Phase = 'idle' | 'connecting' | 'recording' | 'processing' | 'completed' | 'cancelled' | 'failed';
export interface Snapshot {
  protocol: 1;
  revision: number;
  windows: { main: boolean; overlay: boolean };
  session: { id: number; phase: Phase };
}
export type DemoAction = 'start' | 'finish' | 'cancel' | 'fail';
export type WindowAction = 'show' | 'hide' | 'minimize' | 'toggleMaximize';
export type RuntimeErrorCode = 'unavailable' | 'invalidAction' | 'staleSession' | 'forbidden';
export class RuntimeError extends Error {
  constructor(public readonly code: RuntimeErrorCode) { super(code); }
}
export interface DesktopRuntimePort {
  readonly surface: Surface;
  getSnapshot(): Promise<Snapshot>;
  subscribe(listener: (snapshot: Snapshot) => void): Promise<() => void>;
  windowAction(surface: Surface, action: WindowAction): Promise<Snapshot>;
  demo(action: DemoAction, sessionId: number): Promise<Snapshot>;
  dispose(): void;
}
export const initialSnapshot = (): Snapshot => ({ protocol: 1, revision: 0, windows: { main: true, overlay: false }, session: { id: 0, phase: 'idle' } });

// Treat the native/browser boundary as untrusted: reject malformed protocol payloads.
export function parseSnapshot(value: unknown): Snapshot {
  if (typeof value !== 'object' || value === null) throw new RuntimeError('unavailable');
  const s = value as Partial<Snapshot>;
  const phases: Phase[] = ['idle','connecting','recording','processing','completed','cancelled','failed'];
  if (s.protocol !== 1 || !Number.isSafeInteger(s.revision) || s.revision! < 0 ||
      typeof s.windows?.main !== 'boolean' || typeof s.windows?.overlay !== 'boolean' ||
      !Number.isSafeInteger(s.session?.id) || s.session!.id < 0 || !phases.includes(s.session!.phase)) throw new RuntimeError('unavailable');
  return { protocol: 1, revision: s.revision!, windows: { ...s.windows }, session: { ...s.session! } };
}
