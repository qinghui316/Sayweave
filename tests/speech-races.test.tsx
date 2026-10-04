import {it,expect,vi} from 'vitest';
import {render,screen,fireEvent,act} from '@testing-library/react';
import {MemoryRouter} from 'react-router-dom';
import {DeviceSettings} from '../src/features/settings/DeviceSettings';
import {History} from '../src/features/history';
import {initialSpeech,type SpeechPort,type RecordingPreferences} from '../src/contracts/speech';
import type {HistoryPort,HistoryRecord,PlaybackState} from '../src/contracts/history';
import type {ModelConfigPort} from '../src/contracts/models';
vi.mock('../src/ui',()=>({
 Button:({children,onClick,disabled}:{children:React.ReactNode;onClick?:()=>void;disabled?:boolean})=><button disabled={disabled} onClick={onClick}>{children}</button>,
 Field:({label,children}:{label:string;children:(p:object)=>React.ReactNode})=><label>{label}{children({'aria-label':label})}</label>,
 InlineMessage:({children}:{children:React.ReactNode})=><div>{children}</div>,
 Dialog:({children,open}:{children:React.ReactNode;open:boolean})=>open?<div>{children}</div>:null,
 CopyFeedbackButton:()=>null,EmptyState:()=>null,TextInput:(p:React.InputHTMLAttributes<HTMLInputElement>)=><input {...p}/>
}));
function deferred<T>(){let resolve!:(v:T)=>void;const promise=new Promise<T>(r=>resolve=r);return {promise,resolve};}
it('a late device refresh cannot overwrite a committed selection',async()=>{
 const old:RecordingPreferences={deviceId:null,historyEnabled:true,audioEnabled:true,retentionDays:30,revision:1};
 const refresh=deferred<RecordingPreferences>(),save=deferred<RecordingPreferences>();
 const port={mode:'native',devices:vi.fn().mockResolvedValue([{id:'mic',name:'Mic',isDefault:false}]),preferences:vi.fn().mockResolvedValueOnce(old).mockReturnValueOnce(refresh.promise),savePreferences:vi.fn().mockReturnValue(save.promise)} as unknown as SpeechPort;
 render(<DeviceSettings port={port}/>);const select=await screen.findByRole('combobox');
 fireEvent.click(screen.getByRole('button',{name:'刷新设备'}));fireEvent.change(select,{target:{value:'mic'}});
 expect(screen.getByRole('button',{name:'刷新设备'})).toBeDisabled();
 await act(async()=>{save.resolve({...old,deviceId:'mic',revision:2});});expect(select).toHaveValue('mic');
 await act(async()=>{refresh.resolve(old);});expect(select).toHaveValue('mic');
 expect(port.savePreferences).toHaveBeenCalledWith({...old,deviceId:'mic'});
});
it('late playback polling cannot overwrite a newer play action',async()=>{
 vi.useFakeTimers();try{
 const old=deferred<PlaybackState>();const r:HistoryRecord={id:'record',createdAt:1,endedAt:2,durationMs:1000,status:'success',currentAttempt:'attempt',revision:1,audio:{id:'audio',recordId:'record',status:'ready',bytes:32000,expiresAt:null},attempts:[{id:'attempt',recordId:'record',kind:'live',configId:'c',configName:'c',configRevision:1,modelId:'m',serviceHost:'host',createdAt:1,endedAt:2,status:'success',text:'history text',errorCode:null,elapsedMs:1}]};
 const port={mode:'native',status:vi.fn().mockResolvedValue({state:'ready'}),list:vi.fn().mockResolvedValue({records:[r],total:1,page:1,pageSize:50}),detail:vi.fn().mockResolvedValue(r),play:vi.fn().mockResolvedValue({resourceId:'audio',playing:true,positionMs:0,durationMs:1000}),playback:vi.fn().mockReturnValue(old.promise)} as unknown as HistoryPort;
 const speech={preferences:vi.fn().mockResolvedValue({deviceId:null,historyEnabled:true,audioEnabled:true,retentionDays:30,revision:1})} as unknown as SpeechPort;
 render(<MemoryRouter><History port={port} speech={speech} models={{} as ModelConfigPort} snapshot={initialSpeech()}/></MemoryRouter>);
 await act(async()=>vi.advanceTimersByTime(0));await act(async()=>fireEvent.click(screen.getByText('history text')));
 await act(async()=>vi.advanceTimersByTime(250));await act(async()=>fireEvent.click(screen.getByRole('button',{name:'播放录音'})));
 expect(screen.getByRole('button',{name:'暂停'})).toBeVisible();
 await act(async()=>old.resolve({resourceId:null,playing:false,positionMs:0,durationMs:0}));
 expect(screen.getByRole('button',{name:'暂停'})).toBeVisible();
 }finally{vi.useRealTimers();}
});
