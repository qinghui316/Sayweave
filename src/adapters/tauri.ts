import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { parseSnapshot, RuntimeError, type DesktopRuntimePort, type Snapshot, type Surface, type DemoAction, type WindowAction, type RuntimeErrorCode } from '../contracts/runtime';
import { BrowserRuntime } from './browser';

class TauriRuntime implements DesktopRuntimePort {
  readonly surface: Surface;
  private unlisteners = new Set<() => void>();
  constructor() { this.surface = getCurrentWindow().label === 'voice-overlay' ? 'overlay' : 'main'; }
  private async call(command: string, args?: Record<string, unknown>): Promise<Snapshot> {
    try { return parseSnapshot(await invoke(command, args)); }
    catch (error) {
      if (error instanceof RuntimeError) throw error;
      const code = typeof error === 'object' && error !== null && 'code' in error ? String(error.code) : 'unavailable';
      const known: RuntimeErrorCode[] = ['unavailable','invalidAction','staleSession','forbidden'];
      throw new RuntimeError(known.includes(code as RuntimeErrorCode) ? code as RuntimeErrorCode : 'unavailable');
    }
  }
  getSnapshot() { return this.call('runtime_snapshot'); }
  async subscribe(listener: (snapshot: Snapshot) => void) {
    const unlisten = await listen<unknown>('runtime:snapshot', event => {
      try { listener(parseSnapshot(event.payload)); } catch { /* Next explicit read can resynchronize. */ }
    });
    const release = () => { unlisten(); this.unlisteners.delete(release); };
    this.unlisteners.add(release);
    return release;
  }
  windowAction(surface: Surface, action: WindowAction) { return this.call('runtime_window', { surface, action }); }
  demo(action: DemoAction, sessionId: number) { return this.call('runtime_demo', { action, sessionId }); }
  dispose() { this.unlisteners.forEach(release => release()); }
}
export function createRuntime(): DesktopRuntimePort {
  return isTauri() ? new TauriRuntime() : new BrowserRuntime(new URLSearchParams(location.search).get('surface') === 'overlay' ? 'overlay' : 'main');
}
