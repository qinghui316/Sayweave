import {describe,it,expect,vi} from 'vitest';
import {SpeechStore} from './speech-store';
import {BrowserSpeech,BrowserHistory} from '../adapters/speech-browser';
import {initialSpeech,parseSpeech,parsePreferences} from '../contracts/speech';
import {parseRecord} from '../contracts/history';
describe('speech renderer synchronization',()=>{
 it('ignores stale task meters and does not use content updates as lifecycle changes',()=>{
  const store=new SpeechStore(new BrowserSpeech());
  store.accept({...initialSpeech(),sequence:2,lifecycle:2,taskId:'one',phase:'recording'});
  store.meter({taskId:'one',sequence:10,level:.5,durationMs:900});
  expect(store.getSnapshot().snapshot.lifecycle).toBe(2);
  store.accept({...initialSpeech(),sequence:3,lifecycle:3,taskId:'two',phase:'recording'});
  store.meter({taskId:'one',sequence:11,level:1,durationMs:9999});
  expect(store.getSnapshot().snapshot.durationMs).toBe(0);expect(store.getSnapshot().level).toBe(0);
  store.meter({taskId:'two',sequence:1,level:.25,durationMs:100});
  store.accept({...initialSpeech(),sequence:4,lifecycle:4,taskId:'two',phase:'cancelled'});
  store.meter({taskId:'two',sequence:2,level:1,durationMs:100});expect(store.getSnapshot().level).toBe(0);
  store.accept({...initialSpeech(),sequence:3,lifecycle:3,taskId:'two',phase:'recording'});expect(store.getSnapshot().snapshot.phase).toBe('cancelled');
 });
 it('StrictMode overlapping connections keep the newer subscription',async()=>{
  const port=new BrowserSpeech(),store=new SpeechStore(port);const old=await store.connect(),current=await store.connect();old();
  await port.start();expect(store.getSnapshot().snapshot.phase).toBe('connecting');expect(port.listeners.size).toBe(1);current();expect(port.listeners.size).toBe(0);await port.control(port.value.taskId!,'cancel');
 });
 it('cancel during processing discards new simulated history and rejects late completion',async()=>{
  vi.useFakeTimers();try{const port=new BrowserSpeech(),history=new BrowserHistory(port);await port.start();await vi.advanceTimersByTimeAsync(250);await port.control(port.value.taskId!,'finish');await port.control(port.value.taskId!,'cancel');await vi.advanceTimersByTimeAsync(500);expect(port.value.phase).toBe('cancelled');expect((await history.list('',1)).total).toBe(0);await port.start();await vi.advanceTimersByTimeAsync(250);await port.control(port.value.taskId!,'finish');await vi.advanceTimersByTimeAsync(250);expect((await history.list('',1)).total).toBe(1);}finally{vi.useRealTimers();}
 });
 it('rejects malformed event and retention contracts',()=>{
  expect(()=>parsePreferences({deviceId:null,historyEnabled:true,audioEnabled:true,retentionDays:'30',revision:1})).toThrow();
  expect(()=>parseSpeech({...initialSpeech(),segments:[{id:1,text:'missing fields'}]})).toThrow();
  expect(()=>parseRecord({id:'x',createdAt:0,durationMs:0,status:'success',revision:1,currentAttempt:null,endedAt:null,attempts:[],audio:{id:'x',recordId:'x',status:'outside',bytes:0,expiresAt:null}})).toThrow();
 });
});
