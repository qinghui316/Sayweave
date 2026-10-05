import {it,expect,vi,beforeAll,afterAll} from 'vitest';
import userEvent from '@testing-library/user-event';
import {render,renderHook,screen,act} from '@testing-library/react';
import {DeviceSettings} from '../src/features/settings/DeviceSettings';
import {audioLabel} from '../src/features/history/HistoryRow';
import {useHistoryPlayback} from '../src/features/history/usePlayback';
import {initialSpeech,parseSpeech,speechError,type SpeechPort,type RecordingPreferences} from '../src/contracts/speech';
import type {HistoryPort,HistoryRecord,PlaybackState} from '../src/contracts/history';
import {speechMessages} from '../src/locales/speech';
// jsdom has no layout; native menus and scrolling are covered by browser/desktop checks.
const originalScroll=HTMLElement.prototype.scrollIntoView;
beforeAll(()=>{HTMLElement.prototype.scrollIntoView=vi.fn();});
afterAll(()=>{if(originalScroll)HTMLElement.prototype.scrollIntoView=originalScroll;else Reflect.deleteProperty(HTMLElement.prototype,'scrollIntoView');});
function deferred<T>(){let resolve!:(v:T)=>void;const promise=new Promise<T>(r=>resolve=r);return {promise,resolve};}
it('connection timeout survives snapshot parsing separately from recognition timeout',()=>{
 const value={...initialSpeech(),phase:'failed',error:'connectionTimeout'};
 expect(parseSpeech(value).error).toBe('connectionTimeout');
 expect(speechMessages[speechError({code:value.error}).code]).toBe('连接语音服务超时，请重新开始录音。');
 expect(parseSpeech({...value,error:'timeout'}).error).toBe('timeout');
});
it.each([{bytes:0,durationMs:0,label:'没有录下音频'},{bytes:32000,durationMs:1000,label:'录音文件缺失'}])('distinguishes uncaptured audio from a lost recorded file ($label)',async({bytes,durationMs,label})=>{
 const record:HistoryRecord={id:'failed',createdAt:1,endedAt:2,durationMs,status:'failed',currentAttempt:'attempt',revision:1,
  audio:{id:'audio',recordId:'failed',status:'missing',bytes,expiresAt:999999},
  attempts:[{id:'attempt',recordId:'failed',kind:'live',configId:'c',configName:'model',configRevision:1,modelId:'m',serviceHost:'host',createdAt:1,endedAt:2,status:'failed',text:'',errorCode:bytes?'timeout':'connectionTimeout',elapsedMs:5000}]};
 expect(audioLabel(record)).toBe(label);
});
it('a late device refresh cannot overwrite a committed selection',async()=>{
 const old:RecordingPreferences={deviceId:null,historyEnabled:true,audioEnabled:true,retentionDays:30,revision:1};
 const refresh=deferred<RecordingPreferences>(),save=deferred<RecordingPreferences>();
 const port={mode:'native',devices:vi.fn().mockResolvedValue([{id:'mic',name:'Mic',isDefault:false}]),preferences:vi.fn().mockResolvedValueOnce(old).mockReturnValueOnce(refresh.promise),savePreferences:vi.fn().mockReturnValue(save.promise)} as unknown as SpeechPort;
 const user=userEvent.setup();render(<DeviceSettings port={port}/>);const select=await screen.findByRole('combobox');
 await user.click(screen.getByRole('button',{name:'刷新设备'}));select.focus();await user.keyboard('[ArrowDown][End][Enter]');
 expect(screen.getByRole('button',{name:'刷新设备'})).toBeDisabled();
 await act(async()=>{save.resolve({...old,deviceId:'mic',revision:2});});expect(select).toHaveTextContent('Mic');
 await act(async()=>{refresh.resolve(old);});expect(select).toHaveTextContent('Mic');
 expect(port.savePreferences).toHaveBeenCalledWith({...old,deviceId:'mic'});
});
it('late playback polling cannot overwrite a newer play action',async()=>{
 vi.useFakeTimers();try{
 const old=deferred<PlaybackState>();
 const empty={resourceId:null,playing:false,positionMs:0,durationMs:0};
 const playing={resourceId:'audio',playing:true,positionMs:0,durationMs:1000};
 const port={play:vi.fn().mockResolvedValue(playing),playback:vi.fn().mockImplementation((action:string)=>action==='status'?old.promise:Promise.resolve(empty))} as unknown as HistoryPort;
 const {result,rerender}=renderHook(({visible})=>useHistoryPlayback(port,visible),{initialProps:{visible:true}});
 await act(async()=>result.current.control('play','audio'));
 expect(result.current.state.playing).toBe(true);
 await act(async()=>old.resolve(empty));expect(result.current.state.playing).toBe(true);
 await act(async()=>result.current.control('pause'));expect(result.current.state.playing).toBe(false);
 const calls=vi.mocked(port.playback).mock.calls.length;
 await act(async()=>vi.advanceTimersByTime(3000));expect(port.playback).toHaveBeenCalledTimes(calls);
 rerender({visible:false});await act(async()=>vi.advanceTimersByTime(3000));expect(port.playback).toHaveBeenCalledTimes(calls);
 }finally{vi.useRealTimers();}
});
it('finishes a slow play before the departure stop, even across a new mount',async()=>{
 const slow=deferred<PlaybackState>(),order:string[]=[];const empty={resourceId:null,playing:false,positionMs:0,durationMs:0};
 const port={play:vi.fn().mockImplementation(async(id:string)=>{order.push('play:'+id);if(id==='old')return slow.promise;return {...empty,resourceId:id,playing:true};}),playback:vi.fn().mockImplementation(async(action:string)=>{if(action==='stop')order.push('stop');return empty;})} as unknown as HistoryPort;
 const old=renderHook(()=>useHistoryPlayback(port,true));let flight!:Promise<void>;act(()=>{flight=old.result.current.control('play','old');});await act(async()=>{});old.unmount();
 const next=renderHook(()=>useHistoryPlayback(port,true));let newFlight!:Promise<void>;act(()=>{newFlight=next.result.current.control('play','new');});
 await act(async()=>{slow.resolve({...empty,resourceId:'old',playing:true});await flight;await newFlight;});expect(order).toEqual(['play:old','stop','play:new']);expect(next.result.current.state.resourceId).toBe('new');
});
it('an old continuation cannot start playback after the page has left',async()=>{
 const port={play:vi.fn(),playback:vi.fn().mockResolvedValue({resourceId:null,playing:false,positionMs:0,durationMs:0})} as unknown as HistoryPort;
 const view=renderHook(()=>useHistoryPlayback(port,true));const control=view.result.current.control;view.unmount();await expect(control('play','audio')).rejects.toMatchObject({code:'cancelled'});expect(port.play).not.toHaveBeenCalled();
});
