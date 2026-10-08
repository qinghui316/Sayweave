import {act,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {beforeEach,expect,it,vi} from 'vitest';
import {App} from './App';
import {BrowserSpeech,BrowserHistory} from '../adapters/speech-browser';
import {initialSpeech,type SpeechSnapshot} from '../contracts/speech';

let port:BrowserSpeech,history:BrowserHistory;
vi.mock('../adapters/speech-tauri',()=>({createSpeechPorts:()=>({speech:port,history})}));
beforeEach(()=>{port=new BrowserSpeech();history=new BrowserHistory(port);window.location.hash='/';});
function publish(value:Partial<SpeechSnapshot>){
 const sequence=port.value.sequence+1;
 port.value={...initialSpeech(),...value,sequence};
 act(()=>{for(const listener of port.listeners)listener(structuredClone(port.value));});
}
async function mount(){render(<App/>);await waitFor(()=>expect(port.listeners.size).toBe(1));}

it('Home stays idle for external tasks, including errors and meters',async()=>{
 await mount();
 const text=screen.getByRole('region',{name:'识别文字'});
 const original=text.textContent;
 publish({taskId:'outside',source:'external',phase:'recording',text:'外部口述'});
 act(()=>{for(const listener of port.meters)listener({taskId:'outside',sequence:1,captureSequence:1,energies:Array(15).fill(.9),bars:Array(15).fill(.9),level:.9,durationMs:5000});});
 expect(text).toHaveTextContent(original!);expect(screen.queryByRole('button',{name:'结束录音'})).toBeNull();
 expect(screen.getByRole('button',{name:'开始录音'})).toBeDisabled();
 publish({taskId:'outside',source:'external',phase:'failed',text:'外部失败',error:'service'});
 expect(text).toHaveTextContent(original!);expect(screen.queryByRole('alert')).toBeNull();
 expect(screen.getByRole('button',{name:'开始录音'})).toBeEnabled();
});

it('explicit trial updates Home, retains its result across external/history tasks and navigation',async()=>{
 await mount();const control=vi.spyOn(port,'control'),copy=vi.spyOn(port,'copy');
 fireEvent.click(screen.getByRole('button',{name:'开始录音'}));
 await screen.findByRole('button',{name:'结束录音'});
 const trialId=port.value.taskId;
 fireEvent.click(screen.getByRole('button',{name:'结束录音'}));
 await waitFor(()=>expect(port.value.phase).toBe('completed'));
 expect(control).toHaveBeenCalledWith(trialId,'finish');
 const text=screen.getByRole('region',{name:'识别文字'}),retained=text.textContent;
 publish({taskId:'outside',source:'external',phase:'recording',text:'不能覆盖试说',canRetry:true});
 expect(text.textContent).toBe(retained);expect(screen.queryByRole('button',{name:'结束录音'})).toBeNull();
 expect(screen.queryByRole('button',{name:'重新识别'})).toBeNull();
 fireEvent.click(screen.getByRole('button',{name:'复制'}));await waitFor(()=>expect(copy).toHaveBeenCalledWith(retained));
 publish({taskId:'history',source:'history',phase:'processing',text:'历史重识别',error:'timeout'});
 expect(text.textContent).toBe(retained);expect(screen.queryByRole('button',{name:'取消并丢弃'})).toBeNull();
 fireEvent.click(screen.getByRole('link',{name:'历史记录'}));await screen.findByRole('heading',{name:'历史记录'});
 fireEvent.click(screen.getByRole('link',{name:'首页'}));
 expect(screen.getByRole('region',{name:'识别文字'}).textContent).toBe(retained);
 expect(control).toHaveBeenCalledTimes(1);
 publish({taskId:'history',source:'history',phase:'completed',text:'历史结果'});
 await waitFor(()=>expect(screen.getByRole('button',{name:'开始录音'})).toBeEnabled());
});

it('an external current snapshot on frontend reload leaves trial empty',async()=>{
 port.value={...initialSpeech(),sequence:50,taskId:'outside',source:'external',phase:'recording',text:'外部口述'};
 await mount();
 expect(screen.getByRole('region',{name:'识别文字'})).not.toHaveTextContent('外部口述');
 expect(screen.getByRole('button',{name:'开始录音'})).toBeDisabled();
});
it('a missing or reordered trial terminal event cannot leave stale recording controls',async()=>{
 await mount();
 publish({taskId:'trial-a',source:'trial',phase:'recording',text:'试说部分文字'});
 const terminal={...port.value,sequence:port.value.sequence+1,phase:'completed' as const,text:'试说最终文字'};
 port.value={...port.value,sequence:terminal.sequence};
 publish({taskId:'external',source:'external',phase:'recording',text:'外部文字'});
 expect(screen.queryByRole('button',{name:'结束录音'})).toBeNull();
 expect(screen.queryByRole('button',{name:'取消并丢弃'})).toBeNull();
 expect(screen.getByRole('region',{name:'识别文字'})).toHaveTextContent('试说部分文字');
 publish({taskId:'external',source:'external',phase:'completed',text:'外部结果'});
 expect(screen.getByRole('button',{name:'开始录音'})).toBeEnabled();
 act(()=>{for(const listener of port.listeners)listener(terminal);});
 expect(screen.getByRole('region',{name:'识别文字'})).toHaveTextContent('试说最终文字');
 expect(screen.getByRole('button',{name:'开始录音'})).toBeEnabled();
});
