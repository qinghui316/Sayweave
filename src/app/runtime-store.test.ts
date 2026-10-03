import { expect, it, vi } from 'vitest';
import { RuntimeStore } from './runtime-store';
import { BrowserRuntime } from '../adapters/browser';
import { initialSnapshot } from '../contracts/runtime';
it('subscribes before fetch and drops an older read or late event',async()=>{
  const port=new BrowserRuntime(), old=initialSnapshot();
  const newer={...old,revision:9,session:{id:3,phase:'recording' as const}};
  vi.spyOn(port,'subscribe').mockImplementation(async listener=>{listener(newer);return vi.fn<() => void>();});
  vi.spyOn(port,'getSnapshot').mockResolvedValue({...old,revision:2});
  const store=new RuntimeStore(port);await store.connect();expect(store.getSnapshot()).toEqual(newer);
  store.accept({...old,revision:8});expect(store.getSnapshot()).toEqual(newer);
});
