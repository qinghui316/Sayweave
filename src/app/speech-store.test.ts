import {describe,it,expect,vi} from 'vitest';
import {SpeechStore} from './speech-store';
import {BrowserSpeech,BrowserHistory} from '../adapters/speech-browser';
import {initialSpeech,parseSpeech,parsePreferences,parseMeter} from '../contracts/speech';
import {parseRecord} from '../contracts/history';
describe('speech renderer synchronization',()=>{
 it('retains only trial state while global external/history events and meters continue',async()=>{
  const port=new BrowserSpeech(),store=new SpeechStore(port),changed=vi.fn();store.subscribeTrial(changed);
  store.accept({...initialSpeech(),sequence:2,taskId:'external-first',source:'external',phase:'recording',text:'outside'});
  store.meter({taskId:'external-first',sequence:1,captureSequence:1,energies:Array(15).fill(.8),bars:Array(15).fill(.8),level:.8,durationMs:400});
  expect(store.getTrialSnapshot().snapshot).toEqual(initialSpeech());expect(changed).not.toHaveBeenCalled();
  store.accept({...initialSpeech(),sequence:3,taskId:'trial',source:'trial',phase:'recording',text:'practice'});
  store.meter({taskId:'trial',sequence:1,captureSequence:1,energies:Array(15).fill(.4),bars:Array(15).fill(.4),level:.4,durationMs:300});
  expect(store.getTrialSnapshot().level).toBe(.4);
  store.accept({...store.getSnapshot().snapshot,sequence:4,phase:'completed',saved:true,canRetry:true});
  const retained=store.getTrialSnapshot();changed.mockClear();
  for(const [sequence,source] of [[5,'external'],[6,'history']] as const){
   store.accept({...initialSpeech(),sequence,taskId:source,source,phase:'processing',text:source,error:'service'});
   store.meter({taskId:source,sequence:1,captureSequence:1,energies:Array(15).fill(.9),bars:Array(15).fill(.9),level:.9,durationMs:9999});
   expect(store.getSnapshot().snapshot.text).toBe(source);expect(store.getTrialSnapshot()).toBe(retained);
  }
  store.meter({taskId:'trial',sequence:99,captureSequence:99,energies:Array(15).fill(1),bars:Array(15).fill(1),level:1,durationMs:9999});
  port.value={...store.getSnapshot().snapshot,sequence:7,phase:'failed'};await store.resync();
  expect(store.getTrialSnapshot()).toBe(retained);expect(changed).not.toHaveBeenCalled();
  store.accept({...initialSpeech(),sequence:8,taskId:'trial-next',source:'trial',phase:'connecting'});
  expect(store.getTrialSnapshot().snapshot.text).toBe('');expect(store.getTrialSnapshot().level).toBe(0);
 });
 it('reloading during an external task does not restore it into Home',async()=>{
  const port=new BrowserSpeech();port.value={...initialSpeech(),sequence:20,taskId:'outside',source:'external',phase:'recording',text:'outside'};
  const store=new SpeechStore(port),release=await store.connect();
  expect(store.getSnapshot().snapshot.taskId).toBe('outside');expect(store.getTrialSnapshot().snapshot.taskId).toBeNull();release();
 });
 it('late trial termination settles Home without replacing a newer global or trial task',()=>{
  const store=new SpeechStore(new BrowserSpeech());
  store.accept({...initialSpeech(),sequence:10,taskId:'trial-a',source:'trial',phase:'recording',text:'partial'});
  store.accept({...initialSpeech(),sequence:12,taskId:'external',source:'external',phase:'recording'});
  store.accept({...initialSpeech(),sequence:11,taskId:'trial-a',source:'trial',phase:'completed',text:'final'});
  expect(store.getTrialSnapshot().snapshot.text).toBe('final');expect(store.getTrialSnapshot().snapshot.phase).toBe('completed');
  expect(store.getSnapshot().snapshot.taskId).toBe('external');
  store.accept({...initialSpeech(),sequence:13,taskId:'trial-c',source:'trial',phase:'recording',text:'new trial'});
  store.accept({...initialSpeech(),sequence:11,taskId:'trial-a',source:'trial',phase:'completed',text:'final'});
  expect(store.getTrialSnapshot().snapshot.taskId).toBe('trial-c');expect(store.getTrialSnapshot().snapshot.text).toBe('new trial');
 });
 it('ignores stale task meters and does not use content updates as lifecycle changes',()=>{
  const store=new SpeechStore(new BrowserSpeech());
  store.accept({...initialSpeech(),sequence:2,lifecycle:2,taskId:'one',source:'trial',phase:'recording'});
  store.meter({taskId:'one',sequence:10,captureSequence:10,energies:Array(15).fill(.5),bars:Array(15).fill(.5),level:.5,durationMs:900});
  expect(store.getSnapshot().snapshot.lifecycle).toBe(2);
  store.accept({...initialSpeech(),sequence:3,lifecycle:3,taskId:'two',source:'trial',phase:'recording'});
  store.meter({taskId:'one',sequence:11,captureSequence:11,energies:Array(15).fill(1),bars:Array(15).fill(1),level:1,durationMs:9999});
  expect(store.getSnapshot().snapshot.durationMs).toBe(0);expect(store.getSnapshot().level).toBe(0);
  store.meter({taskId:'two',sequence:1,captureSequence:1,energies:Array(15).fill(.25),bars:Array(15).fill(.25),level:.25,durationMs:100});
  store.accept({...initialSpeech(),sequence:4,lifecycle:4,taskId:'two',source:'trial',phase:'cancelled'});
  store.meter({taskId:'two',sequence:2,captureSequence:2,energies:Array(15).fill(1),bars:Array(15).fill(1),level:1,durationMs:100});expect(store.getSnapshot().level).toBe(0);
  store.accept({...initialSpeech(),sequence:3,lifecycle:3,taskId:'two',source:'trial',phase:'recording'});expect(store.getSnapshot().snapshot.phase).toBe('cancelled');
 });
 it('StrictMode overlapping connections keep the newer subscription',async()=>{
  const port=new BrowserSpeech(),store=new SpeechStore(port);const old=await store.connect(),current=await store.connect();old();
  await port.start();expect(store.getSnapshot().snapshot.phase).toBe('connecting');expect(port.listeners.size).toBe(1);current();expect(port.listeners.size).toBe(0);await port.control(port.value.taskId!,'cancel');
 });
 it('cancel during processing discards new simulated history and rejects late completion',async()=>{
  vi.useFakeTimers();try{const port=new BrowserSpeech(),history=new BrowserHistory(port);await port.start();await vi.advanceTimersByTimeAsync(250);await port.control(port.value.taskId!,'finish');await port.control(port.value.taskId!,'cancel');await vi.advanceTimersByTimeAsync(500);expect(port.value.phase).toBe('cancelled');expect((await history.list('',1)).total).toBe(0);await port.start();await vi.advanceTimersByTimeAsync(250);await port.control(port.value.taskId!,'finish');await vi.advanceTimersByTimeAsync(250);expect((await history.list('',1)).total).toBe(1);}finally{vi.useRealTimers();}
 });
 it('rejects malformed event and retention contracts',()=>{
  for(const source of ['trial','external','history'] as const)expect(parseSpeech({...initialSpeech(),taskId:'task',source}).source).toBe(source);
  expect(()=>parseSpeech({...initialSpeech(),taskId:'task'})).toThrow();
  expect(()=>parseSpeech({...initialSpeech(),source:undefined})).toThrow();
  expect(()=>parseSpeech({...initialSpeech(),source:'trial'})).toThrow();
  expect(()=>parseSpeech({...initialSpeech(),taskId:'task',source:'unknown'})).toThrow();
  expect(()=>parsePreferences({deviceId:null,historyEnabled:true,audioEnabled:true,retentionDays:'30',revision:1})).toThrow();
  expect(()=>parseSpeech({...initialSpeech(),segments:[{id:1,text:'missing fields'}]})).toThrow();
  expect(()=>parseRecord({id:'x',createdAt:0,durationMs:0,status:'success',revision:1,currentAttempt:null,endedAt:null,attempts:[],audio:{id:'x',recordId:'x',status:'outside',bytes:0,expiresAt:null}})).toThrow();
 });
 it('rejected simulated actions do not cancel the accepted transition',async()=>{
  vi.useFakeTimers();try{const port=new BrowserSpeech();const first=await port.start();await expect(port.start()).rejects.toMatchObject({code:'busy'});await expect(port.control(first.taskId!,'finish')).rejects.toMatchObject({code:'invalid'});await vi.advanceTimersByTimeAsync(250);expect(port.value.phase).toBe('recording');await port.control(first.taskId!,'cancel');}finally{vi.useRealTimers();}
 });
});

it('applies late archive fields to their retained trial without replacing a new external task',()=>{
 const store=new SpeechStore(new BrowserSpeech());
 store.accept({...initialSpeech(),sequence:10,taskId:'trial-a',source:'trial',phase:'completed',text:'保留试说'});
 store.accept({...initialSpeech(),sequence:11,taskId:'external-b',source:'external',phase:'recording',text:'外部口述'});
 store.archived({taskId:'trial-a',saved:true,saveError:false});
 expect(store.getTrialSnapshot().snapshot).toMatchObject({text:'保留试说',saved:true,phase:'completed'});
 expect(store.getSnapshot().snapshot).toMatchObject({taskId:'external-b',text:'外部口述',saved:false});
 store.accept({...initialSpeech(),sequence:12,taskId:'trial-c',source:'trial',phase:'recording',text:'新的试说'});
 store.archived({taskId:'trial-a',saved:false,saveError:true});
 expect(store.getTrialSnapshot().snapshot).toMatchObject({taskId:'trial-c',text:'新的试说',saveError:false});
});

it('meter frames notify only the waveform, publishing page time once per second',()=>{
 const store=new SpeechStore(new BrowserSpeech());store.accept({...initialSpeech(),sequence:2,taskId:'trial',source:'trial',phase:'recording'});
 const page=vi.fn(),trial=vi.fn(),wave=vi.fn();store.subscribe(page);store.subscribeTrial(trial);store.trialMeter.subscribe(wave);
 for(let i=1;i<=20;i++)store.meter({taskId:'trial',sequence:i,captureSequence:i,energies:Array(15).fill(.1),bars:Array(15).fill(.1),level:.1,durationMs:i*50});
 expect(wave).toHaveBeenCalledTimes(20);expect(page).toHaveBeenCalledTimes(1);expect(trial).toHaveBeenCalledTimes(1);
 const current=store.trialMeter.getSnapshot();store.meter({taskId:'trial',sequence:21,captureSequence:20,energies:Array(15).fill(1),bars:Array(15).fill(1),level:1,durationMs:1050});
 expect(store.trialMeter.getSnapshot()).toBe(current);expect(wave).toHaveBeenCalledTimes(20);
 expect(()=>parseMeter({...current,bars:[0]})).toThrow();expect(()=>parseMeter({...current,bars:Array(15).fill(Infinity)})).toThrow();expect(()=>parseMeter({...current,energies:[0]})).toThrow();expect(()=>parseMeter({...current,energies:Array(15).fill(NaN),bars:Array(15).fill(NaN)})).toThrow();
});
