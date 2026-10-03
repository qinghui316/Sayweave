import { initialSnapshot, RuntimeError, type DesktopRuntimePort, type Snapshot, type Phase, type Surface, type DemoAction, type WindowAction } from '../contracts/runtime';

export class BrowserRuntime implements DesktopRuntimePort {
  readonly surface: Surface;
  private state = initialSnapshot();
  private listeners = new Set<(snapshot: Snapshot) => void>();
  private timers = new Set<ReturnType<typeof setTimeout>>();
  constructor(surface: Surface = 'main') { this.surface = surface; }
  async getSnapshot() { return structuredClone(this.state); }
  async subscribe(listener: (snapshot: Snapshot) => void) {
    // Each connection owns its registration, including overlapping StrictMode effects.
    const registration = (snapshot: Snapshot) => listener(snapshot);
    this.listeners.add(registration);
    return () => { this.listeners.delete(registration); };
  }
  private publish() {
    this.state = { ...this.state, revision: this.state.revision + 1 };
    for (const listener of this.listeners) listener(structuredClone(this.state));
    return structuredClone(this.state);
  }
  private later(id: number, expected: Phase, phase: Phase, delay: number) {
    const timer = setTimeout(() => {
      this.timers.delete(timer);
      if (this.state.session.id !== id || this.state.session.phase !== expected) return;
      this.state.session = { id, phase };
      this.publish();
    }, delay);
    this.timers.add(timer);
  }
  async demo(action: DemoAction, sessionId: number) {
    const current = this.state.session;
    if (action === 'start') {
      if (['connecting','recording','processing'].includes(current.phase)) throw new RuntimeError('invalidAction');
      const id = current.id + 1;
      this.state.session = { id, phase: 'connecting' };
      this.later(id, 'connecting', 'recording', 650);
    } else {
      if (sessionId !== current.id) throw new RuntimeError('staleSession');
      if (action === 'finish' && current.phase === 'recording') {
        this.state.session = { ...current, phase: 'processing' };
        this.later(current.id, 'processing', 'completed', 900);
        this.later(current.id, 'completed', 'idle', 1900);
      } else if (['cancel','fail'].includes(action) && ['connecting','recording','processing'].includes(current.phase)) {
        this.state.session = { ...current, phase: action === 'cancel' ? 'cancelled' : 'failed' };
      } else throw new RuntimeError('invalidAction');
    }
    return this.publish();
  }
  async windowAction(surface: Surface, action: WindowAction) {
    if (this.surface === 'overlay' && (surface !== 'overlay' || action !== 'hide')) throw new RuntimeError('forbidden');
    if (action === 'show' || action === 'hide') this.state.windows = { ...this.state.windows, [surface]: action === 'show' };
    return this.publish();
  }
  dispose() { this.timers.forEach(clearTimeout); this.timers.clear(); this.listeners.clear(); }
}
