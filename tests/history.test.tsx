import {act,fireEvent,render,screen,waitFor,within} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {it,expect,vi} from 'vitest';
import {MemoryRouter} from 'react-router-dom';
import {History,HistoryFeedStore} from '../src/features/history';
import {BrowserHistory,BrowserSpeech} from '../src/adapters/speech-browser';
import {BrowserModels} from '../src/adapters/models-browser';
import {parseFeed,summarizeHistory,type HistoryRecord} from '../src/contracts/history';
import {initialSpeech} from '../src/contracts/speech';
import {SaveSettings,type DepartureGuard} from '../src/features/history/SaveSettings';
function record(n=1,text='完整的识别文字'):HistoryRecord {const id=`record-${n}`,a=`attempt-${n}`;return {id,createdAt:1000+n,endedAt:2000+n,durationMs:1000,status:'success',currentAttempt:a,revision:1,audio:null,attempts:[{id:a,recordId:id,kind:'live',configId:'fixture',configName:'模型',configRevision:1,modelId:'fixture',serviceHost:'example.invalid',createdAt:1000+n,endedAt:2000+n,status:'success',text,errorCode:null,elapsedMs:1}]};}
function fixture(records:HistoryRecord[]){const speech=new BrowserSpeech();speech.records.push(...records);return {speech,port:new BrowserHistory(speech),models:new BrowserModels()};}
function deferred<T>(){let resolve!:(value:T)=>void;const promise=new Promise<T>(r=>resolve=r);return {promise,resolve};}
it('refreshes late delivery confirmation without changing the recognition record',async()=>{
 const r=record(),f=fixture([r]);let state='delivering';
 const port=Object.assign(Object.create(f.port) as import('../src/contracts/history').HistoryPort,{delivery:vi.fn(async()=>[{attemptId:r.currentAttempt!,state,error:null,updatedAt:1}])});
 const view=render(<MemoryRouter><History port={port} speech={f.speech} models={f.models} snapshot={initialSpeech()} deliveryRevision="working"/></MemoryRouter>);
 await screen.findByText('完整的识别文字');const user=userEvent.setup();screen.getByRole('button',{name:'更多操作'}).focus();await user.keyboard('{Enter}');await user.click(screen.getByRole('menuitem',{name:'识别版本'}));
 await screen.findByText('填入未完成');state='confirmed';
 view.rerender(<MemoryRouter><History port={port} speech={f.speech} models={f.models} snapshot={initialSpeech()} deliveryRevision="confirmed"/></MemoryRouter>);
 expect(await screen.findByText('已填入')).toBeVisible();
});
it('loads 1,000 summaries by cursor, survives deletion of an anchor, and retains reading state',async()=>{
 const f=fixture(Array.from({length:1000},(_,n)=>record(n)));const store=new HistoryFeedStore(f.port);
 await store.refresh();expect(store.getSnapshot().records).toHaveLength(50);const cursor=store.getSnapshot().cursor!;
 await f.port.remove(cursor.id,1);store.remove(cursor.id);
 while(store.getSnapshot().cursor)await store.more();
 expect(store.getSnapshot().records).toHaveLength(999);expect(new Set(store.getSnapshot().records.map(r=>r.id)).size).toBe(999);
 store.setScroll(720);store.expand('record-700');await store.refresh();
 expect(store.getScroll()).toBe(720);expect(store.getSnapshot().expandedId).toBe('record-700');expect(store.getSnapshot().cursor).toBeNull();
 f.speech.records.push(record(2000));await store.refresh();expect(store.getSnapshot().newRecords).toBe(true);expect(store.getSnapshot().records.some(r=>r.id==='record-2000')).toBe(false);
});
it('discards stale searches and a late page containing a deleted row',async()=>{
 const f=fixture([record(1,'旧字'),record(2,'新字')]);const store=new HistoryFeedStore(f.port);const old=deferred<Awaited<ReturnType<BrowserHistory['browse']>>>();
 const browse=vi.spyOn(f.port,'browse').mockReturnValueOnce(old.promise);
 const initial=store.refresh();await Promise.resolve();store.search('新字');await store.refresh();
 old.resolve({records:[summarizeHistory(record(1,'旧字'))!],total:1,nextCursor:null});await initial;
 expect(store.getSnapshot().records.map(r=>r.id)).toEqual(['record-2']);
 browse.mockRestore();
 const f2=fixture(Array.from({length:70},(_,n)=>record(n)));const s2=new HistoryFeedStore(f2.port);await s2.refresh();
 const late=deferred<Awaited<ReturnType<BrowserHistory['browse']>>>();vi.spyOn(f2.port,'browse').mockReturnValueOnce(late.promise);const more=s2.more();
 s2.remove('record-69');late.resolve({records:[summarizeHistory(record(69))!],total:70,nextCursor:null});await more;
 expect(s2.getSnapshot().records.some(r=>r.id==='record-69')).toBe(false);
});
it('does not skip a gap when more than fifty new records arrive at the top',async()=>{
 const f=fixture(Array.from({length:100},(_,n)=>record(n)));const store=new HistoryFeedStore(f.port);await store.refresh();
 f.speech.records.push(...Array.from({length:60},(_,n)=>record(100+n)));await store.refresh();while(store.getSnapshot().cursor)await store.more();
 expect(store.getSnapshot().records).toHaveLength(160);expect(new Set(store.getSnapshot().records.map(r=>r.id)).size).toBe(160);
 const retry=vi.spyOn(f.port,'status');await store.refresh(undefined,true);expect(retry).toHaveBeenLastCalledWith(true);
 expect(summarizeHistory(record(1,'旧文字'),' 旧文字 ')?.preview).toBe('旧文字');
});
it('validates bounded summaries and distinguishes a failed retry without losing success',()=>{
 const r=record(1,'当前成功');r.attempts.unshift({...r.attempts[0]!,id:'failed-retry',kind:'retry',createdAt:9000,status:'failed',text:'',errorCode:'timeout'});
 const summary=summarizeHistory(r)!;expect(summary.preview).toBe('当前成功');expect(summary.latestError).toBe('timeout');
 expect(parseFeed({records:[summary],total:1,nextCursor:null}).records[0]!.displayAttempt).toBe(r.currentAttempt);
 expect(()=>parseFeed({records:[{...summary,preview:'😀'.repeat(241)}],total:1,nextCursor:null})).toThrow();
 expect(()=>parseFeed({records:Array(51).fill(summary),total:51,nextCursor:null})).toThrow();
});
it('copies the full matched old version and opens versions inline with keyboard access',async()=>{
 const r=record(1,'当前成功');const full='旧文字😀'.repeat(90);r.attempts.push({...r.attempts[0]!,id:'old',createdAt:1,text:full});
 const f=fixture([r]),copy=vi.spyOn(f.speech,'copy').mockResolvedValue();
 render(<MemoryRouter><History {...f} snapshot={initialSpeech()}/></MemoryRouter>);
 await screen.findByText('当前成功');fireEvent.change(screen.getByRole('textbox',{name:'搜索识别文字'}),{target:{value:'旧文字'}});
 await screen.findByText('匹配旧版本');const row=screen.getByRole('listitem');fireEvent.click(within(row).getByRole('button',{name:'复制'}));
 await waitFor(()=>expect(copy).toHaveBeenCalledWith(full));expect(within(row).getByRole('button',{name:'已复制'})).toBeVisible();
 const user=userEvent.setup();within(row).getByRole('button',{name:'更多操作'}).focus();await user.keyboard('{Enter}');await user.click(screen.getByRole('menuitem',{name:'识别版本'}));
 expect(await screen.findByRole('region',{name:'历史识别文字'})).toHaveTextContent(full);expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
 expect(screen.getByRole('combobox',{name:'识别版本'})).toHaveValue('old');
});
it('keeps a successful body beside a failed retry and deletes only after inline confirmation',async()=>{
 const r=record();r.attempts.unshift({...r.attempts[0]!,id:'retry',kind:'retry',createdAt:9000,status:'failed',text:'',errorCode:'timeout'});const f=fixture([r]);
 render(<MemoryRouter><History {...f} snapshot={initialSpeech()}/></MemoryRouter>);await screen.findByText('完整的识别文字');expect(screen.getByText(/本次重识别未完成/)).toBeVisible();
 const user=userEvent.setup();screen.getByRole('button',{name:'更多操作'}).focus();await user.keyboard('{Enter}');await user.click(screen.getByRole('menuitem',{name:'删除记录'}));
 const confirm=await screen.findByRole('group',{name:'删除历史确认'});expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
 await user.click(within(confirm).getByRole('button',{name:'取消'}));expect(f.speech.records).toHaveLength(1);
 expect(screen.getByRole('button',{name:'更多操作'})).toHaveFocus();
 screen.getByRole('button',{name:'更多操作'}).focus();await user.keyboard('{Enter}');await user.click(screen.getByRole('menuitem',{name:'删除记录'}));
 await user.click(within(await screen.findByRole('group',{name:'删除历史确认'})).getByRole('button',{name:'确认删除'}));
 await screen.findByText('还没有历史记录');expect(f.speech.records).toHaveLength(0);
});
it('a delayed full-text read cannot replace a newer copy or write after leaving',async()=>{
 const f=fixture([record(1,'较早文字'),record(2,'较新文字')]),copy=vi.spyOn(f.speech,'copy').mockResolvedValue();
 const view=render(<MemoryRouter><History {...f} snapshot={initialSpeech()}/></MemoryRouter>);await screen.findByText('较新文字');
 const delayed=deferred<HistoryRecord>();vi.spyOn(f.port,'detail').mockReturnValueOnce(delayed.promise);
 const rows=screen.getAllByRole('listitem');fireEvent.click(within(rows[1]!).getByRole('button',{name:'复制'}));fireEvent.click(within(rows[0]!).getByRole('button',{name:'复制'}));await waitFor(()=>expect(copy).toHaveBeenCalledWith('较新文字'));
 await act(async()=>delayed.resolve(record(1,'较早文字')));expect(copy).toHaveBeenCalledTimes(1);expect(within(rows[1]!).getByRole('button',{name:'复制'})).toBeVisible();
 const leaving=deferred<HistoryRecord>();vi.spyOn(f.port,'detail').mockReturnValueOnce(leaving.promise);fireEvent.click(within(rows[1]!).getByRole('button',{name:'复制'}));view.unmount();await act(async()=>leaving.resolve(record(1,'较早文字')));expect(copy).toHaveBeenCalledTimes(1);
});
it('protects unsaved retention and preserves a concurrent microphone selection on save',async()=>{
 const speech=new BrowserSpeech();let guard:DepartureGuard|null=null;render(<SaveSettings speech={speech} onGuardChange={v=>{guard=v;}}/>);
 const select=await screen.findByRole('combobox',{name:'录音保留时间'});fireEvent.change(select,{target:{value:'7'}});
 const leave=vi.fn();act(()=>guard!(leave));expect(leave).not.toHaveBeenCalled();fireEvent.click(screen.getByRole('button',{name:'继续填写'}));expect(select).toHaveValue('7');
 const dirtyGuard=guard;speech.settings={...speech.settings,deviceId:'new-device',revision:2};fireEvent.click(screen.getByRole('button',{name:'保存设置'}));
 await screen.findByText('已保存，下一次录音生效。');expect(speech.settings).toMatchObject({deviceId:'new-device',retentionDays:7,revision:3});
 // onGuardChange is an effect contract; observe its rebind before invoking it.
 await waitFor(()=>{expect(guard).not.toBeNull();expect(guard).not.toBe(dirtyGuard);});
 act(()=>guard!(leave));expect(leave).toHaveBeenCalledOnce();
});
it('can retry unavailable save settings without leaving the history page',async()=>{
 const speech=new BrowserSpeech();vi.spyOn(speech,'preferences').mockRejectedValueOnce({code:'unavailable'});
 render(<SaveSettings speech={speech}/>);fireEvent.click(await screen.findByRole('button',{name:'重试读取设置'}));
 expect(await screen.findByRole('combobox',{name:'录音保留时间'})).toHaveValue('30');
});
