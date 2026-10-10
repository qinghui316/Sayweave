import {expect,it} from 'vitest';
import {SettingStore} from '../src/settings/store';
it('update barrier retains and waits for admitted saves while rejecting new intents',async()=>{
 let persisted={revision:1,enabled:false},resolve!:(value:typeof persisted)=>void;
 const store=new SettingStore(async()=>persisted,async()=>new Promise<typeof persisted>(done=>{resolve=done;}));
 await store.load();store.update({enabled:true});store.pause();store.update({enabled:false});
 let finished=false;const flush=store.flush().then(()=>{finished=true;});await Promise.resolve();expect(finished).toBe(false);
 persisted={revision:2,enabled:true};resolve(persisted);await flush;expect(store.getSnapshot().value).toEqual(persisted);
 store.resume();store.update({enabled:false});persisted={revision:3,enabled:false};resolve(persisted);await store.flush();expect(store.getSnapshot().value?.enabled).toBe(false);
});
it('an uncertain persisted setting blocks installation until a true read reconciles it',async()=>{
 let reads=0;const store=new SettingStore(async()=>{if(++reads>1)throw new Error('offline');return{revision:1,enabled:false};},async()=>{throw new Error('lost IPC');});
 await store.load();store.update({enabled:true});store.pause();await expect(store.flush()).rejects.toThrow();expect(store.getSnapshot().value?.enabled).toBe(true);
});
