import { afterEach, describe, expect, it, vi } from 'vitest';
import { BrowserRuntime } from './browser';
describe('Browser runtime contract',()=>{
  afterEach(()=>vi.useRealTimers());
  it('keeps a newer connection when an overlapping effect releases the same listener',async()=>{
    const port=new BrowserRuntime(), listener=vi.fn();
    const old=await port.subscribe(listener), current=await port.subscribe(listener);
    old();await port.windowAction('overlay','show');expect(listener).toHaveBeenCalledTimes(1);
    current();await port.windowAction('overlay','hide');expect(listener).toHaveBeenCalledTimes(1);port.dispose();
  });
  it('rejects late processing and old sessions after cancellation',async()=>{
    vi.useFakeTimers(); const port=new BrowserRuntime();
    let s=await port.demo('start',0); const old=s.session.id; await vi.advanceTimersByTimeAsync(650);
    await port.demo('finish',old); await port.demo('cancel',old); s=await port.demo('start',old);
    await vi.advanceTimersByTimeAsync(2000); expect((await port.getSnapshot()).session).toEqual({id:s.session.id,phase:'recording'});
    await expect(port.demo('finish',old)).rejects.toMatchObject({code:'staleSession'}); port.dispose();
  });
  it('releases listeners and preserves session when main hidden',async()=>{
    vi.useFakeTimers(); const port=new BrowserRuntime(), listener=vi.fn(); const off=await port.subscribe(listener);
    await port.demo('start',0); await port.windowAction('main','hide'); await vi.advanceTimersByTimeAsync(650);
    expect((await port.getSnapshot()).session.phase).toBe('recording'); off();const count=listener.mock.calls.length;
    await port.demo('cancel',1);expect(listener).toHaveBeenCalledTimes(count);port.dispose();
  });
  it('forbids overlay managing main window',async()=>{
    const port=new BrowserRuntime('overlay');await expect(port.windowAction('main','show')).rejects.toMatchObject({code:'forbidden'});port.dispose();
  });
});
