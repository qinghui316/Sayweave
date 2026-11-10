import {recordingSettings} from './settings-fixtures';
import {act,fireEvent,render,screen,waitFor,within} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {it,expect,vi} from 'vitest';
import {MemoryRouter} from 'react-router-dom';
import {History,HistoryFeedStore} from '../src/features/history';
import {BrowserHistory,BrowserSpeech} from '../src/adapters/speech-browser';
import {parseFeed,parseRecord,summarizeHistory,type HistoryRecord} from '../src/contracts/history';
import {initialSpeech} from '../src/contracts/speech';
import {SaveSettings} from '../src/features/history/SaveSettings';
function record(n=1,text='完整的识别文字'):HistoryRecord {const id=`record-${n}`,a=`attempt-${n}`;return {id,createdAt:1000+n,endedAt:2000+n,durationMs:1000,status:'success',currentAttempt:a,revision:1,audio:null,attempts:[{id:a,recordId:id,kind:'live',configId:'fixture',configName:'模型',configRevision:1,modelId:'fixture',serviceHost:'example.invalid',createdAt:1000+n,endedAt:2000+n,status:'success',text,errorCode:null,elapsedMs:1}]};}
function fixture(records:HistoryRecord[]){const speech=new BrowserSpeech();speech.records.push(...records);return {speech,port:new BrowserHistory(speech)};}
function deferred<T>(){let resolve!:(value:T)=>void;const promise=new Promise<T>(r=>resolve=r);return {promise,resolve};}
it('automatically follows saved recordings and late processed summaries without replacing the row identity',async()=>{
 const f=fixture([]),store=new HistoryFeedStore(f.port);await store.refresh();
 f.speech.records.push(record(1,'第一轮原文'));await store.refresh();
 expect(store.getSnapshot().records.map(r=>r.preview)).toEqual(['第一轮原文']);
 const first=store.getSnapshot().records[0]!,epoch=store.getSnapshot().epoch;
 f.speech.records[0]!.processing=[{id:'p',attemptId:first.displayAttempt!,configName:'文字模型',modelId:'fixture',state:'success',text:'第一轮整理',revision:1,errorCode:null,units:[]}];
 await store.refresh();expect(store.getSnapshot().records[0]).toMatchObject({id:first.id,preview:'第一轮整理'});expect(store.getSnapshot().epoch).toBe(epoch);
 f.speech.records.push(record(2,'第二轮'));await store.refresh();await store.refresh();
 expect(store.getSnapshot().records.map(r=>r.id)).toEqual(['record-2','record-1']);
});
it('resumes automatically after scrolling, expanding or releasing the last reading hold',async()=>{
 const f=fixture([record(1)]),store=new HistoryFeedStore(f.port);await store.refresh();store.setScroll(80);
 f.speech.records.push(record(2));await store.refresh();expect(store.getSnapshot().records.map(r=>r.id)).toEqual(['record-1']);expect(store.getScroll()).toBe(80);
 store.setScroll(0);await waitFor(()=>expect(store.getSnapshot().records[0]?.id).toBe('record-2'));
 store.expand('record-2');f.speech.records.push(record(3));await store.refresh();
 expect(store.getSnapshot().expandedId).toBe('record-2');expect(store.getSnapshot().records[0]?.id).toBe('record-2');
 store.expand(null);await waitFor(()=>expect(store.getSnapshot().records[0]?.id).toBe('record-3'));
 const releaseCopy=store.hold(),releasePlayback=store.hold();f.speech.records.push(record(4));await store.refresh();releaseCopy();releaseCopy();
 expect(store.getSnapshot().records[0]?.id).toBe('record-3');releasePlayback();
 await waitFor(()=>expect(store.getSnapshot().records[0]?.id).toBe('record-4'));
});
it('rebuilds forward cursors after following resumes without duplicating or dropping records',async()=>{
 const f=fixture(Array.from({length:41},(_,n)=>record(n))),store=new HistoryFeedStore(f.port);await store.refresh();await store.next();
 const older=store.getSnapshot().records.map(r=>r.id);f.speech.records.push(record(100));await store.refresh();
 expect(store.getSnapshot().records.map(r=>r.id)).toEqual(older);await store.previous();
 await waitFor(()=>expect(store.getSnapshot().records[0]?.id).toBe('record-100'));
 const ids:string[]=[];
 ids.push(...store.getSnapshot().records.map(r=>r.id));while(store.getSnapshot().cursor){await store.next();ids.push(...store.getSnapshot().records.map(r=>r.id));}
 expect(ids).toHaveLength(42);expect(new Set(ids).size).toBe(42);expect(new Set(ids)).toEqual(new Set(f.speech.records.map(r=>r.id)));
});
it('rechecks reading eligibility after a slow head read and preserves data across a failed refresh',async()=>{
 const f=fixture([record(1)]),store=new HistoryFeedStore(f.port);await store.refresh();f.speech.records.push(record(2));
 const head=await f.port.browse('',null,20),late=deferred<typeof head>(),browse=vi.spyOn(f.port,'browse').mockReturnValueOnce(late.promise);
 const pending=store.refresh();await waitFor(()=>expect(browse).toHaveBeenCalledOnce());store.setScroll(100);late.resolve(head);await pending;
 expect(store.getSnapshot().records[0]?.id).toBe('record-1');expect(store.getScroll()).toBe(100);
 browse.mockRejectedValueOnce({code:'unavailable'});store.setScroll(0);await waitFor(()=>expect(store.getSnapshot().error).toBe('unavailable'));
 expect(store.getSnapshot().records[0]?.id).toBe('record-1');expect(store.getSnapshot().loaded).toBe(true);
 await store.retry();expect(store.getSnapshot().records[0]?.id).toBe('record-2');expect(store.getSnapshot().error).toBeNull();
});
it('resumes pending history when an older automatic read is superseded before returning to the top',async()=>{
 const f=fixture([record(1)]),store=new HistoryFeedStore(f.port);await store.refresh();store.setScroll(80);
 f.speech.records.push(record(2));await store.refresh();
 const head=await f.port.browse('',null,20),late=deferred<typeof head>(),browse=vi.spyOn(f.port,'browse').mockReturnValueOnce(late.promise);
 store.setScroll(0);await waitFor(()=>expect(browse).toHaveBeenCalledOnce());store.setScroll(80);
 f.speech.records.push(record(3));await store.refresh();store.setScroll(0);
 late.resolve(head);await waitFor(()=>expect(store.getSnapshot().records.map(r=>r.id)).toEqual(['record-3','record-2','record-1']));
});
it('retains a resume requested while the current automatic read is finishing a reading hold',async()=>{
 const f=fixture([record(1)]),store=new HistoryFeedStore(f.port);await store.refresh();store.expand('record-1');
 f.speech.records.push(record(2));await store.refresh();
 const head=await f.port.browse('',null,20),late=deferred<typeof head>(),browse=vi.spyOn(f.port,'browse').mockReturnValueOnce(late.promise);
 store.expand(null);await waitFor(()=>expect(browse).toHaveBeenCalledOnce());const release=store.hold();let releaseOnRead=true;
 const unsubscribe=store.subscribe(()=>{if(releaseOnRead){releaseOnRead=false;release();}});
 late.resolve(head);await waitFor(()=>expect(store.getSnapshot().records[0]?.id).toBe('record-2'));unsubscribe();
});
it('ignores a late resumed head after a newer search and preserves the current query',async()=>{
 const f=fixture([record(1,'旧文字')]),store=new HistoryFeedStore(f.port);await store.refresh();store.setScroll(80);
 f.speech.records.push(record(2,'新文字'));await store.refresh();
 const head=await f.port.browse('',null,20),late=deferred<typeof head>(),browse=vi.spyOn(f.port,'browse').mockReturnValueOnce(late.promise);
 store.setScroll(0);await waitFor(()=>expect(browse).toHaveBeenCalledOnce());store.search('新文字');await store.refresh();late.resolve(head);
 await waitFor(()=>expect(store.getSnapshot().records.map(r=>r.id)).toEqual(['record-2']));expect(store.getSnapshot().search).toBe('新文字');
 f.speech.records.push(record(3,'不匹配'),record(4,'新文字后续'));await store.refresh();expect(store.getSnapshot().records.map(r=>r.id)).toEqual(['record-4','record-2']);
});
it('renders newly archived and processed text automatically without a latest button',async()=>{
 const f=fixture([]),props={...f,recordingSettings:recordingSettings(f.speech),snapshot:initialSpeech()};
 const view=render(<MemoryRouter><History {...props}/></MemoryRouter>);await screen.findByText('还没有历史记录');
 f.speech.records.push(record(1,'新口述'));view.rerender(<MemoryRouter><History {...props} contentRevision={1}/></MemoryRouter>);
 expect(await screen.findByText('新口述')).toBeVisible();expect(screen.queryByRole('button',{name:/查看最新/})).not.toBeInTheDocument();
 const r=f.speech.records[0]!;r.processing=[{id:'translated',attemptId:r.currentAttempt!,configName:'文字模型',modelId:'fixture',state:'success',text:'New translation',revision:1,errorCode:null,units:[]}];
 view.rerender(<MemoryRouter><History {...props} contentRevision={2}/></MemoryRouter>);
 expect(await screen.findByText('New translation')).toBeVisible();expect(screen.getAllByRole('listitem')).toHaveLength(1);
});
it('keeps a copy read bound to its row while new history arrives and resumes after copying',async()=>{
 const f=fixture([record(1,'正在复制的文字')]),copy=vi.spyOn(f.speech,'copy').mockResolvedValue(),props={...f,recordingSettings:recordingSettings(f.speech),snapshot:initialSpeech()};
 const view=render(<MemoryRouter><History {...props}/></MemoryRouter>);await screen.findByText('正在复制的文字');
 const detail=deferred<HistoryRecord>();vi.spyOn(f.port,'detail').mockReturnValueOnce(detail.promise);fireEvent.click(screen.getByRole('button',{name:'复制'}));
 f.speech.records.push(record(2,'随后完成的口述'));view.rerender(<MemoryRouter><History {...props} contentRevision={1}/></MemoryRouter>);
 await waitFor(()=>expect(screen.getByText('2 条')).toBeVisible());expect(screen.queryByText('随后完成的口述')).not.toBeInTheDocument();
 await act(async()=>detail.resolve(record(1,'正在复制的文字')));await waitFor(()=>expect(copy).toHaveBeenCalledWith('正在复制的文字'));
 expect(await screen.findByText('随后完成的口述')).toBeVisible();
});
it('releases an obsolete copy hold when a newer copy completes or history is left',async()=>{
 const f=fixture([record(1,'第一条'),record(2,'第二条')]),store=new HistoryFeedStore(f.port),copy=vi.spyOn(f.speech,'copy').mockResolvedValue(),props={...f,reading:store,recordingSettings:recordingSettings(f.speech),snapshot:initialSpeech()};
 const view=render(<MemoryRouter><History {...props}/></MemoryRouter>);await screen.findByText('第二条');
 const stale=deferred<HistoryRecord>(),detail=vi.spyOn(f.port,'detail').mockReturnValueOnce(stale.promise);
 fireEvent.click(within(screen.getAllByRole('listitem')[1]!).getByRole('button',{name:'复制'}));
 fireEvent.click(within(screen.getAllByRole('listitem')[0]!).getByRole('button',{name:'复制'}));await waitFor(()=>expect(copy).toHaveBeenCalledWith('第二条'));
 f.speech.records.push(record(3,'第三条'));view.rerender(<MemoryRouter><History {...props} contentRevision={1}/></MemoryRouter>);expect(await screen.findByText('第三条')).toBeVisible();
 const leaving=deferred<HistoryRecord>();detail.mockReturnValueOnce(leaving.promise);fireEvent.click(within(screen.getAllByRole('listitem')[0]!).getByRole('button',{name:'复制'}));view.unmount();
 f.speech.records.push(record(4,'第四条'));render(<MemoryRouter><History {...props}/></MemoryRouter>);expect(await screen.findByText('第四条')).toBeVisible();
 await act(async()=>{stale.resolve(record(1,'第一条'));leaving.resolve(record(3,'第三条'));});expect(copy).toHaveBeenCalledTimes(1);
});
it('copies and searches the full processed version independently of raw text',async()=>{
 const r=record(1,'识别原文'),full='整理后的完整正文🙂'.repeat(50);
 const refs=[{segment:1,revision:1,chars:{start:0,end:4}}];
 const unit={id:'unit',runId:'processed-1',turn:1,sourceStart:0,sourceEnd:4,sourceText:'识别原文',candidateText:full,generationState:'success',applicationState:'applied',sourceCommittedEnd:4,errorCode:null,promptTokens:null,completionTokens:null,elapsedMs:1};
 r.processing=[{id:'processed-1',attemptId:r.currentAttempt!,configName:'文字模型',modelId:'fixture',state:'success',text:full,revision:1,errorCode:null,units:[{...unit,sourceRefs:JSON.stringify({format:'revisable-v1',order:1,source:{block:'7e580f4a-6d40-4980-b590-c068c9ff7736',version:1,sourceEpoch:1,baselineKind:'asr',text:'识别原文',speechEnd:4,insertionEnd:4},asrTurn:1,asrSources:refs,delivery:false,visibleRevision:null})}]}];
 expect(parseRecord(r).processing?.[0]?.text).toBe(full);
 expect(parseRecord({...r,processing:[{...r.processing[0]!,units:[{...unit,sourceRefs:JSON.stringify(refs)}]}]}).processing?.[0]?.text).toBe(full);
 const f=fixture([r]),copy=vi.spyOn(f.speech,'copy').mockResolvedValue();
 const detail=f.port.detail.bind(f.port);vi.spyOn(f.port,'detail').mockImplementation(async id=>parseRecord(await detail(id)));
 render(<MemoryRouter><History {...f} recordingSettings={recordingSettings(f.speech)} snapshot={initialSpeech()}/></MemoryRouter>);
 const row=await screen.findByRole('listitem');fireEvent.click(within(row).getByRole('button',{name:'展开'}));
 expect(await within(row).findByRole('region',{name:'历史识别文字'})).toHaveTextContent(full);
 fireEvent.click(within(row).getByRole('button',{name:'复制'}));
 await waitFor(()=>expect(copy).toHaveBeenCalledWith(full));
 fireEvent.click(screen.getByRole('button',{name:'搜索识别文字'}));fireEvent.change(screen.getByRole('textbox',{name:'搜索识别文字'}),{target:{value:'识别原文'}});
 await screen.findByText('识别原文');fireEvent.click(within(screen.getByRole('listitem')).getByRole('button',{name:'复制'}));
 await waitFor(()=>expect(copy).toHaveBeenLastCalledWith('识别原文'));
});
it('plays without reading details and keeps playback through reading, copy and cancelled deletion',async()=>{
 const a=record(2,'长中文识别文字。'.repeat(40)),b=record(1,'另一段录音');
 for(const r of [a,b])r.audio={id:'audio-'+r.id,recordId:r.id,status:'ready',bytes:32000,expiresAt:null};
 const f=fixture([a,b]),copy=vi.spyOn(f.speech,'copy').mockResolvedValue(),detail=vi.spyOn(f.port,'detail');
 const empty={resourceId:null,playing:false,positionMs:0,durationMs:0};
 let state:import('../src/contracts/history').PlaybackState=empty;
 const play=vi.spyOn(f.port,'play').mockImplementation(async(resourceId?:string)=>state={resourceId:resourceId!,playing:true,positionMs:0,durationMs:1000});
 const playback=vi.spyOn(f.port,'playback').mockImplementation(async action=>{
  if(action==='stop')state=empty;
  if(action==='pause'||action==='resume')state={...state,playing:action==='resume'};
  return state;
 });
 const view=render(<MemoryRouter><History {...f} recordingSettings={recordingSettings(f.speech)} snapshot={initialSpeech()}/></MemoryRouter>);
 const [first,second]=(await screen.findAllByRole('listitem')).map(node=>within(node));
 fireEvent.click(first!.getByRole('button',{name:'播放录音'}));await first!.findByRole('button',{name:'暂停录音'});
 expect(detail).not.toHaveBeenCalled();expect(play).toHaveBeenLastCalledWith(a.audio!.id);
 fireEvent.click(first!.getByRole('button',{name:'暂停录音'}));fireEvent.click(await first!.findByRole('button',{name:'继续播放'}));await first!.findByRole('button',{name:'暂停录音'});
 fireEvent.click(first!.getByRole('button',{name:'展开'}));await first!.findByRole('region',{name:'历史识别文字'});
 fireEvent.click(first!.getByRole('button',{name:'复制'}));await waitFor(()=>expect(copy).toHaveBeenCalledWith(a.attempts[0]!.text));
 fireEvent.click(first!.getByRole('button',{name:'收起'}));
 fireEvent.click(first!.getByRole('button',{name:'删除记录'}));fireEvent.click(first!.getByRole('button',{name:'取消'}));
 expect(first!.getByRole('button',{name:'暂停录音'})).toBeEnabled();expect(playback).not.toHaveBeenCalledWith('stop',undefined);
 fireEvent.click(second!.getByRole('button',{name:'播放录音'}));await second!.findByRole('button',{name:'暂停录音'});
 expect(first!.getByRole('button',{name:'播放录音'})).toBeEnabled();expect(play).toHaveBeenLastCalledWith(b.audio!.id);
 view.unmount();await waitFor(()=>expect(playback).toHaveBeenCalledWith('stop'));
});
it('separates unreadable text from a missing audio file',async()=>{
 const r=record(1,'有识别结果。'.repeat(60));r.audio={id:'missing-on-disk',recordId:r.id,status:'ready',bytes:32000,expiresAt:null};
 const f=fixture([r]),detail=vi.spyOn(f.port,'detail').mockRejectedValue({code:'protocol'});
 render(<MemoryRouter><History {...f} recordingSettings={recordingSettings(f.speech)} snapshot={initialSpeech()}/></MemoryRouter>);
 const row=within(await screen.findByRole('listitem'));fireEvent.click(row.getByRole('button',{name:'播放录音'}));
 expect(await row.findByText('录音文件缺失或无法读取。')).toBeVisible();expect(detail).not.toHaveBeenCalled();
 fireEvent.click(row.getByRole('button',{name:'展开'}));expect(await row.findByText('无法读取这条历史记录。')).toBeVisible();
});
it('does not start audio from a departing row during slow pagination',async()=>{
 const newest=record(20,'即将离开的录音');
 newest.audio={id:'departing-audio',recordId:newest.id,status:'ready',bytes:32000,expiresAt:null};
 const f=fixture([...Array.from({length:20},(_,n)=>record(n,'第'+n+'条记录')),newest]);
 const original=f.port.browse.bind(f.port),head=await original('',null,20);
 const next=await original('',head.nextCursor,20),pending=deferred<typeof next>();
 vi.spyOn(f.port,'browse').mockImplementation((search,cursor,limit)=>cursor?pending.promise:original(search,cursor,limit));
 const play=vi.spyOn(f.port,'play').mockResolvedValue({resourceId:newest.audio.id,playing:true,positionMs:0,durationMs:1000});
 render(<MemoryRouter><History {...f} recordingSettings={recordingSettings(f.speech)} snapshot={initialSpeech()}/></MemoryRouter>);
 await screen.findByText('即将离开的录音');
 const button=screen.getByRole('button',{name:'播放录音'});
 expect(button).toBeEnabled();
 fireEvent.click(screen.getByRole('button',{name:'下一页'}));
 await screen.findByText('正在翻页…');
 await act(async()=>{fireEvent.click(button);});
 expect(play).not.toHaveBeenCalled();
 expect(button).toBeDisabled();
 await act(async()=>pending.resolve(next));
 expect(await screen.findByText('第0条记录')).toBeVisible();
 expect(screen.queryByRole('button',{name:'播放录音'})).not.toBeInTheDocument();
});
it('rejects processing records with unrelated source identity or invalid receipts',()=>{
 const r=record();r.processing=[{id:'p',attemptId:r.currentAttempt!,configName:'m',modelId:'m',state:'success',text:'整理',revision:1,errorCode:null,units:[]}];
 expect(()=>parseRecord({...r,processing:[{...r.processing![0],attemptId:'another'}]})).toThrow();
 expect(()=>parseRecord({...r,processing:[{...r.processing![0],units:[{id:'u',runId:'other'}]}]})).toThrow();
});
it('shows retained native draft results without inventing a text model',async()=>{
 const r=record(1,'识别原文');
 r.processing=[{id:'native-draft',attemptId:r.currentAttempt!,mode:'native',configName:'口述结果',modelId:'',state:'success',text:'用户修改后的结果'.repeat(40),revision:1,errorCode:null,units:[]}];
 expect(parseRecord(r).processing?.[0]?.mode).toBe('native');
 expect(()=>parseRecord({...r,processing:[{...r.processing![0],mode:'unknown'}]})).toThrow();
 const f=fixture([r]);
 render(<MemoryRouter><History {...f} recordingSettings={recordingSettings(f.speech)} snapshot={initialSpeech()}/></MemoryRouter>);
 await screen.findByRole('listitem');
 await userEvent.click(screen.getByRole('button',{name:'展开'}));
 expect((await screen.findAllByText('口述结果')).length).toBeGreaterThan(0);
});
it('refreshes late delivery confirmation without changing the recognition record',async()=>{
 const r=record(1,'完整的识别文字'.repeat(40)),f=fixture([r]);let state='delivering';
 const port=Object.assign(Object.create(f.port) as import('../src/contracts/history').HistoryPort,{delivery:vi.fn(async()=>[{attemptId:r.currentAttempt!,state,error:null,updatedAt:1}])});
 const view=render(<MemoryRouter><History port={port} recordingSettings={recordingSettings(f.speech)} speech={f.speech} snapshot={initialSpeech()} deliveryRevision="working"/></MemoryRouter>);
 await userEvent.click(await screen.findByRole('button',{name:'展开'}));
 await screen.findByText('填入未完成');state='confirmed';
 view.rerender(<MemoryRouter><History port={port} recordingSettings={recordingSettings(f.speech)} speech={f.speech} snapshot={initialSpeech()} deliveryRevision="confirmed"/></MemoryRouter>);
 expect(await screen.findByText('已填入')).toBeVisible();
});
it('keeps 20-item reading boundaries across deletion, new records and cache eviction',async()=>{
 const f=fixture(Array.from({length:1000},(_,n)=>record(n))),store=new HistoryFeedStore(f.port);
 const reread=vi.spyOn(f.port,'reread');await store.refresh();expect(store.getSnapshot().records).toHaveLength(20);const release=store.hold();
 const first=store.getSnapshot().records.map(r=>r.id),cursor=store.getSnapshot().cursor!;
 await f.port.remove(cursor.id,1);store.remove(cursor.id);await store.refresh();
 expect(store.getSnapshot().records).toHaveLength(19);expect(store.getSnapshot().cursor).toEqual(cursor);
 const seen=new Set(store.getSnapshot().records.map(r=>r.id));
 while(store.getSnapshot().cursor){await store.next();for(const r of store.getSnapshot().records){expect(seen.has(r.id)).toBe(false);seen.add(r.id);}}
 expect(seen.size).toBe(999);
 store.setScroll(720);await store.refresh();expect(store.getScroll()).toBe(720);
 f.speech.records.push(record(2000));await store.refresh();expect(store.getSnapshot().records.some(r=>r.id==='record-2000')).toBe(false);
 await store.go(0);expect(reread).toHaveBeenCalledWith('',first.filter(id=>id!==cursor.id));
 expect(store.getSnapshot().records.map(r=>r.id)).toEqual(first.filter(id=>id!==cursor.id));
 await store.latest();expect(store.getSnapshot().page).toBe(1);expect(store.getSnapshot().records[0]?.id).toBe('record-2000');expect(store.getSnapshot().records).toHaveLength(20);release();
});
it('discards stale searches and a late page containing a deleted row',async()=>{
 const f=fixture([record(1,'旧字'),record(2,'新字')]);const store=new HistoryFeedStore(f.port);const old=deferred<Awaited<ReturnType<BrowserHistory['browse']>>>();
 const browse=vi.spyOn(f.port,'browse').mockReturnValueOnce(old.promise);
 const initial=store.refresh();await waitFor(()=>expect(browse).toHaveBeenCalledOnce());store.search('新字');await store.refresh();
 old.resolve({records:[summarizeHistory(record(1,'旧字'))!],total:1,nextCursor:null});await initial;
 expect(store.getSnapshot().records.map(r=>r.id)).toEqual(['record-2']);
 browse.mockRestore();
 const f2=fixture(Array.from({length:70},(_,n)=>record(n)));const s2=new HistoryFeedStore(f2.port);await s2.refresh();
 const late=deferred<Awaited<ReturnType<BrowserHistory['browse']>>>();vi.spyOn(f2.port,'browse').mockReturnValueOnce(late.promise);const more=s2.next();
 s2.remove('record-69');late.resolve({records:[summarizeHistory(record(69))!],total:70,nextCursor:null});await more;
 expect(s2.getSnapshot().records.some(r=>r.id==='record-69')).toBe(false);
});
it('keeps incoming records out of protected reading pages and retains explicit refresh',async()=>{
 const f=fixture(Array.from({length:21},(_,n)=>record(n))),store=new HistoryFeedStore(f.port);await store.refresh();
 const release=store.hold(),original=store.getSnapshot().records.map(r=>r.id);f.speech.records.push(...Array.from({length:60},(_,n)=>record(100+n)));
 await store.refresh();expect(store.getSnapshot().records.map(r=>r.id)).toEqual(original);expect(store.getSnapshot().total).toBe(81);
 await store.next();expect(store.getSnapshot().records.map(r=>r.id)).toEqual(['record-0']);
 await f.port.remove('record-0',1);store.remove('record-0');await waitFor(()=>expect(store.getSnapshot().page).toBe(1));
 expect(store.getSnapshot().records.map(r=>r.id)).toEqual(original);
 const retry=vi.spyOn(f.port,'status');await store.latest();expect(retry).toHaveBeenLastCalledWith(true);expect(store.getSnapshot().total).toBe(80);release();
});
it('validates bounded summaries and distinguishes a failed retry without losing success',()=>{
 const r=record(1,'当前成功');r.attempts.unshift({...r.attempts[0]!,id:'failed-retry',kind:'retry',createdAt:9000,status:'failed',text:'',errorCode:'timeout'});
 const summary=summarizeHistory(r)!;expect(summary.preview).toBe('当前成功');expect(summary.latestError).toBe('timeout');
 expect(parseFeed({records:[summary],total:1,nextCursor:null}).records[0]!.displayAttempt).toBe(r.currentAttempt);
 expect(()=>parseFeed({records:[{...summary,preview:'😀'.repeat(241)}],total:1,nextCursor:null})).toThrow();
 expect(()=>parseFeed({records:Array(51).fill(summary),total:51,nextCursor:null})).toThrow();
});
it('copies the full matched old version and expands it inline with keyboard access',async()=>{
 const r=record(1,'当前成功');const full='旧文字😀'.repeat(90);r.attempts.push({...r.attempts[0]!,id:'old',createdAt:1,text:full});
 const f=fixture([r]),copy=vi.spyOn(f.speech,'copy').mockResolvedValue();
 render(<MemoryRouter><History {...f} recordingSettings={recordingSettings(f.speech)} snapshot={initialSpeech()}/></MemoryRouter>);
 await screen.findByText('当前成功');fireEvent.click(screen.getByRole('button',{name:'搜索识别文字'}));fireEvent.change(screen.getByRole('textbox',{name:'搜索识别文字'}),{target:{value:'旧文字'}});
 await screen.findByText('匹配旧版本');const row=screen.getByRole('listitem');fireEvent.click(within(row).getByRole('button',{name:'复制'}));
 await waitFor(()=>expect(copy).toHaveBeenCalledWith(full));expect(within(row).getByRole('button',{name:'已复制'})).toBeVisible();
 const user=userEvent.setup();within(row).getByRole('button',{name:'展开'}).focus();await user.keyboard('{Enter}');
 expect(await screen.findByRole('region',{name:'历史识别文字'})).toHaveTextContent(full);expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
});
it('keeps a successful body beside a failed retry and deletes only after inline confirmation',async()=>{
 const r=record();r.attempts.unshift({...r.attempts[0]!,id:'retry',kind:'retry',createdAt:9000,status:'failed',text:'',errorCode:'timeout'});const f=fixture([r]);
 render(<MemoryRouter><History {...f} recordingSettings={recordingSettings(f.speech)} snapshot={initialSpeech()}/></MemoryRouter>);await screen.findByText('完整的识别文字');expect(screen.getByText(/本次重识别未完成/)).toBeVisible();
 const user=userEvent.setup();screen.getByRole('button',{name:'删除记录'}).focus();await user.keyboard('{Enter}');
 const confirm=await screen.findByRole('group',{name:'删除历史确认'});expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
 await user.click(within(confirm).getByRole('button',{name:'取消'}));expect(f.speech.records).toHaveLength(1);
 expect(screen.getByRole('button',{name:'删除记录'})).toHaveFocus();
 screen.getByRole('button',{name:'删除记录'}).focus();await user.keyboard('{Enter}');
 await user.click(within(await screen.findByRole('group',{name:'删除历史确认'})).getByRole('button',{name:'确认删除'}));
 await screen.findByText('还没有历史记录');expect(f.speech.records).toHaveLength(0);
});
it('a delayed full-text read cannot replace a newer copy or write after leaving',async()=>{
 const f=fixture([record(1,'较早文字'),record(2,'较新文字')]),copy=vi.spyOn(f.speech,'copy').mockResolvedValue();
 const view=render(<MemoryRouter><History {...f} recordingSettings={recordingSettings(f.speech)} snapshot={initialSpeech()}/></MemoryRouter>);await screen.findByText('较新文字');
 const delayed=deferred<HistoryRecord>();vi.spyOn(f.port,'detail').mockReturnValueOnce(delayed.promise);
 const rows=screen.getAllByRole('listitem');fireEvent.click(within(rows[1]!).getByRole('button',{name:'复制'}));fireEvent.click(within(rows[0]!).getByRole('button',{name:'复制'}));await waitFor(()=>expect(copy).toHaveBeenCalledWith('较新文字'));
 await act(async()=>delayed.resolve(record(1,'较早文字')));expect(copy).toHaveBeenCalledTimes(1);expect(within(rows[1]!).getByRole('button',{name:'复制'})).toBeVisible();
 const leaving=deferred<HistoryRecord>();vi.spyOn(f.port,'detail').mockReturnValueOnce(leaving.promise);fireEvent.click(within(rows[1]!).getByRole('button',{name:'复制'}));view.unmount();await act(async()=>leaving.resolve(record(1,'较早文字')));expect(copy).toHaveBeenCalledTimes(1);
});
it('saves retention immediately and retains a concurrently queued microphone change',async()=>{
 const speech=new BrowserSpeech(),settings=recordingSettings(speech);render(<SaveSettings settings={settings}/>);
 const select=await screen.findByRole('combobox',{name:'录音保留时间'});await waitFor(()=>expect(select).toBeEnabled());const user=userEvent.setup();
 act(()=>settings.update({deviceId:'new-device'}));select.focus();await user.keyboard('{Enter}');await user.click(screen.getByRole('option',{name:'7 天'}));
 await waitFor(()=>expect(speech.settings).toMatchObject({deviceId:'new-device',retentionDays:7}));expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();expect(screen.queryByRole('button',{name:'保存设置'})).not.toBeInTheDocument();
});
it('can retry unavailable save settings without leaving the history page',async()=>{
 const speech=new BrowserSpeech();vi.spyOn(speech,'preferences').mockRejectedValueOnce({code:'unavailable'});
 render(<SaveSettings settings={recordingSettings(speech)}/>);fireEvent.click(await screen.findByRole('button',{name:'重新读取'}));
 await waitFor(()=>expect(screen.getByRole('combobox',{name:'录音保留时间'})).toHaveTextContent('30 天'));
});

it('coalesces a background notification received during navigation and handles empty stores',async()=>{
 for(const count of [0,1,20,21]){const f=fixture(Array.from({length:count},(_,n)=>record(n))),store=new HistoryFeedStore(f.port);await store.refresh();expect(store.getSnapshot().records).toHaveLength(Math.min(20,count));expect(!!store.getSnapshot().cursor).toBe(count>20);}
 const f=fixture(Array.from({length:41},(_,n)=>record(n))),store=new HistoryFeedStore(f.port);await store.refresh();
 const page=await f.port.browse('',store.getSnapshot().cursor,20),wait=deferred<typeof page>();vi.spyOn(f.port,'browse').mockReturnValueOnce(wait.promise);
 const navigation=store.next();f.speech.records.push(record(100));await store.refresh();wait.resolve(page);await navigation;
 expect(store.getSnapshot().page).toBe(2);expect(store.getSnapshot().total).toBe(42);expect(store.getSnapshot().records.some(r=>r.id==='record-100')).toBe(false);
});
it('rereads search membership by identity without filling a shrinking page',async()=>{
 const f=fixture(Array.from({length:61},(_,n)=>record(n,'匹配'))),store=new HistoryFeedStore(f.port);store.search('匹配');await store.refresh();
 const ids=store.getSnapshot().records.map(r=>r.id);await store.next();await store.next();await store.next();
 f.speech.records.find(r=>r.id===ids[0])!.attempts[0]!.text='已修改';await store.refresh();await store.go(0);
 expect(store.getSnapshot().records.map(r=>r.id)).toEqual(ids.slice(1));expect(store.getSnapshot().total).toBe(60);
});

it('skips an emptied middle bookmark without refilling or hiding later pages',async()=>{
 const f=fixture(Array.from({length:60},(_,n)=>record(n))),store=new HistoryFeedStore(f.port);await store.refresh();await store.next();
 const removed=store.getSnapshot().records.map(r=>r.id);
 for(const id of removed){await f.port.remove(id,1);store.remove(id);}
 await store.refresh();await waitFor(()=>expect(store.getSnapshot().page).toBe(1));
 await store.next();expect(store.getSnapshot().page).toBe(3);expect(store.getSnapshot().records).toHaveLength(20);expect(store.getSnapshot().records[0]?.id).toBe('record-19');
 await store.previous();expect(store.getSnapshot().page).toBe(1);
 await store.next();expect(store.getSnapshot().page).toBe(3);
});
it('preserves navigation failure and its retry target when a pending update is reconciled',async()=>{
 const f=fixture(Array.from({length:41},(_,n)=>record(n))),store=new HistoryFeedStore(f.port);await store.refresh();
 let fail!:(e:unknown)=>void;const delayed=new Promise<never>((_,reject)=>{fail=reject;});vi.spyOn(f.port,'browse').mockReturnValueOnce(delayed);
 const navigation=store.next();f.speech.records.push(record(100));await store.refresh();fail({code:'unavailable'});await navigation;
 expect(store.getSnapshot()).toMatchObject({page:1,total:42,error:'unavailable'});
 await store.retry();expect(store.getSnapshot().page).toBe(2);expect(store.getSnapshot().error).toBeNull();
});

it('seeks forward when the first page is emptied and shows empty only after the remaining pages disappear',async()=>{
 const f=fixture(Array.from({length:21},(_,n)=>record(n))),store=new HistoryFeedStore(f.port);await store.refresh();
 const release=store.hold();
 for(const row of store.getSnapshot().records){await f.port.remove(row.id,1);store.remove(row.id);}
 await store.refresh();await waitFor(()=>expect(store.getSnapshot().records.map(r=>r.id)).toEqual(['record-0']));
 expect(store.getSnapshot().page).toBe(2);await f.port.remove('record-0',1);store.remove('record-0');await store.refresh();
 expect(store.getSnapshot().records).toHaveLength(0);expect(store.getSnapshot().total).toBe(0);expect(store.getSnapshot().cursor).toBeNull();release();
});

it.each(['service','emptyText'])('shows translation error %s while retaining the recognized original',async errorCode=>{
 const r=record(1,'今天下午三点开会'.repeat(40));r.attempts[0]!.purpose='translation';r.attempts[0]!.targetLanguage='en-US';
 r.processing=[{id:'failed-translation',attemptId:r.currentAttempt!,configName:'文字模型',modelId:'fixture',state:'failed',text:'',revision:1,errorCode,units:[]}];
 const f=fixture([r]),user=userEvent.setup();
 render(<MemoryRouter><History {...f} recordingSettings={recordingSettings(f.speech)} snapshot={initialSpeech()}/></MemoryRouter>);
 await user.click(await screen.findByRole('button',{name:'展开'}));
 expect(await screen.findByRole('region',{name:'历史识别文字'})).toHaveTextContent('今天下午三点开会'.repeat(40));
 expect(screen.getByText(errorCode==='emptyText'?'未生成译文。':'翻译未完成。')).toBeVisible();
});
