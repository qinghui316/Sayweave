import { initialSnapshot, type DesktopRuntimePort, type Snapshot } from '../contracts/runtime';

export class RuntimeStore {
  private snapshot = initialSnapshot();
  private listeners = new Set<() => void>();
  constructor(readonly port: DesktopRuntimePort) {}
  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  accept = (snapshot: Snapshot) => {
    if (snapshot.revision <= this.snapshot.revision) return;
    this.snapshot = snapshot;
    this.listeners.forEach(listener => listener());
  };
  async connect() {
    // Listen before fetch: an event arriving during fetch cannot be overwritten by an older read.
    const release = await this.port.subscribe(this.accept);
    try { this.accept(await this.port.getSnapshot()); return release; }
    catch (error) { release(); throw error; }
  }
  async resync() { this.accept(await this.port.getSnapshot()); }
}
