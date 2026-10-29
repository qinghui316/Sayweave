import {recordingSettings} from './settings-fixtures';
import {act,fireEvent,render,screen,waitFor,within} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {it,expect,vi} from 'vitest';
import {MemoryRouter} from 'react-router-dom';
import {History,HistoryFeedStore} from '../src/features/history';
import {BrowserHistory,BrowserSpeech} from '../src/adapters/speech-browser';
import {BrowserModels} from '../src/adapters/models-browser';
import {parseFeed,parseRecord,summarizeHistory,type HistoryRecord} from '../src/contracts/history';
import {initialSpeech} from '../src/contracts/speech';
import {SaveSettings} from '../src/features/history/SaveSettings';
import {localAssetId} from '../src/contracts/models';
function record(n=1,text='完整的识别文字'):HistoryRecord {const id=`record-${n}`,a=`attempt-${n}`;return {id,createdAt:1000+n,endedAt:2000+n,durationMs:1000,status:'success',currentAttempt:a,revision:1,audio:null,attempts:[{id:a,recordId:id,kind:'live',configId:'fixture',configName:'模型',configRevision:1,modelId:'fixture',serviceHost:'example.invalid',createdAt:1000+n,endedAt:2000+n,status:'success',text,errorCode:null,elapsedMs:1}]};}
function fixture(records:HistoryRecord[]){const speech=new BrowserSpeech();speech.records.push(...records);return {speech,port:new BrowserHistory(speech),models:new BrowserModels()};}
function deferred<T>(){let resolve!:(value:T)=>void;const promise=new Promise<T>(r=>resolve=r);return {promise,resolve};}
it('retries retained audio with a local default without claiming a cloud upload',async()=>{
 const r=record(),f=fixture([r]);r.audio={id:'retained-audio',recordId:r.id,status:'ready',bytes:32000,expiresAt:null};
 await f.models.local.download();
 const view=await f.models.local.save({input:{name:'本地识别',assetId:localAssetId,engine:'transcribe-cpp-0.1.3',device:'cpu'},id:null,revision:null});
 const model=view.localConfigs![0]!;
 await f.models.setDefault({usage:'speech',configId:model.id,revision:view.defaults.find(d=>d.usage==='speech')!.revision});
 const retry=vi.spyOn(f.port,'retry').mockResolvedValue(initialSpeech());
 render(<MemoryRouter><History {...f} recordingSettings={recordingSettings(f.speech)} snapshot={initialSpeech()}/></MemoryRouter>);
 await screen.findByRole('listitem');
 const user=userEvent.setup();screen.getByRole('button',{name:'更多操作'}).focus();await user.keyboard('{Enter}');
 await user.click(await screen.findByRole('menuitem',{name:'重新识别'}));
 const confirm=await screen.findByRole('group',{name:'重识别确认'});
 expect(confirm).toHaveTextContent('本地识别');expect(confirm).not.toHaveTextContent('会发送这份录音');
 await user.click(within(confirm).getByRole('button',{name:'开始识别'}));
 await waitFor(()=>expect(retry).toHaveBeenCalledWith(r.id,model.id,model.revision));
});
it('copies and searches the full processed version independently of raw text',async()=>{
 const r=record(1,'识别原文'),full='整理后的完整正文🙂'.repeat(50);
 r.processing=[{id:'processed-1',attemptId:r.currentAttempt!,configName:'文字模型',modelId:'fixture',state:'success',text:full,revision:1,errorCode:null,units:[]}];
 expect(parseRecord(r).processing?.[0]?.text).toBe(full);
 const f=fixture([r]),copy=vi.spyOn(f.speech,'copy').mockResolvedValue();
 render(<MemoryRouter><History {...f} recordingSettings={recordingSettings(f.speech)} snapshot={initialSpeech()}/></MemoryRouter>);
 const row=await screen.findByRole('listitem');fireEvent.click(within(row).getByRole('button',{name:'复制'}));
 await waitFor(()=>expect(copy).toHaveBeenCalledWith(full));
 fireEvent.click(screen.getByRole('button',{name:'搜索识别文字'}));fireEvent.change(screen.getByRole('textbox',{name:'搜索识别文字'}),{target:{value:'识别原文'}});
 await screen.findByText('识别原文');fireEvent.click(within(screen.getByRole('listitem')).getByRole('button',{name:'复制'}));
 await waitFor(()=>expect(copy).toHaveBeenLastCalledWith('识别原文'));
});
it('rejects processing records with unrelated source identity or invalid receipts',()=>{
 const r=record();r.processing=[{id:'p',attemptId:r.currentAttempt!,configName:'m',modelId:'m',state:'success',text:'整理',revision:1,errorCode:null,units:[]}];
 expect(()=>parseRecord({...r,processing:[{...r.processing![0],attemptId:'another'}]})).toThrow();
 expect(()=>parseRecord({...r,processing:[{...r.processing![0],units:[{id:'u',runId:'other'}]}]})).toThrow();
});
it('shows retained native draft results without inventing a text model',async()=>{
 const r=record(1,'识别原文');
 r.processing=[{id:'native-draft',attemptId:r.currentAttempt!,mode:'native',configName:'口述结果',modelId:'',state:'success',text:'用户修改后的结果',revision:1,errorCode:null,units:[]}];
 expect(parseRecord(r).processing?.[0]?.mode).toBe('native');
 expect(()=>parseRecord({...r,processing:[{...r.processing![0],mode:'unknown'}]})).toThrow();
 const f=fixture([r]);
 render(<MemoryRouter><History {...f} recordingSettings={recordingSettings(f.speech)} snapshot={initialSpeech()}/></MemoryRouter>);
 expect(await screen.findByText('用户修改后的结果')).toBeVisible();
 const user=userEvent.setup();screen.getByRole('button',{name:'更多操作'}).focus();await user.keyboard('{Enter}');
 await user.click(await screen.findByRole('menuitem',{name:'识别版本'}));
 expect((await screen.findAllByText('口述结果')).length).toBeGreaterThan(0);
});
it('refreshes late delivery confirmation without changing the recognition record',async()=>{
 const r=record(),f=fixture([r]);let state='delivering';
 const port=Object.assign(Object.create(f.port) as import('../src/contracts/history').HistoryPort,{delivery:vi.fn(async()=>[{attemptId:r.currentAttempt!,state,error:null,updatedAt:1}])});
 const view=render(<MemoryRouter><History port={port} recordingSettings={recordingSettings(f.speech)} speech={f.speech} models={f.models} snapshot={initialSpeech()} deliveryRevision="working"/></MemoryRouter>);
 await screen.findByText('完整的识别文字');const user=userEvent.setup();screen.getByRole('button',{name:'更多操作'}).focus();await user.keyboard('{Enter}');await user.click(screen.getByRole('menuitem',{name:'识别版本'}));
 await screen.findByText('填入未完成');state='confirmed';
 view.rerender(<MemoryRouter><History port={port} recordingSettings={recordingSettings(f.speech)} speech={f.speech} models={f.models} snapshot={initialSpeech()} deliveryRevision="confirmed"/></MemoryRouter>);
 expect(await screen.findByText('已填入')).toBeVisible();
});
it('keeps 20-item reading boundaries across deletion, new records and cache eviction',async()=>{
 const f=fixture(Array.from({length:1000},(_,n)=>record(n))),store=new HistoryFeedStore(f.port);
 const reread=vi.spyOn(f.port,'reread');await store.refresh();expect(store.getSnapshot().records).toHaveLength(20);
 const first=store.getSnapshot().records.map(r=>r.id),cursor=store.getSnapshot().cursor!;
 await f.port.remove(cursor.id,1);store.remove(cursor.id);await store.refresh();
 expect(store.getSnapshot().records).toHaveLength(19);expect(store.getSnapshot().cursor).toEqual(cursor);
 const seen=new Set(store.getSnapshot().records.map(r=>r.id));
 while(store.getSnapshot().cursor){await store.next();for(const r of store.getSnapshot().records){expect(seen.has(r.id)).toBe(false);seen.add(r.id);}}
 expect(seen.size).toBe(999);
 store.setScroll(720);await store.refresh();expect(store.getScroll()).toBe(720);
 f.speech.records.push(record(2000));await store.refresh();expect(store.getSnapshot().newRecords).toBe(true);
 await store.go(0);expect(reread).toHaveBeenCalledWith('',first.filter(id=>id!==cursor.id));
 expect(store.getSnapshot().records.map(r=>r.id)).toEqual(first.filter(id=>id!==cursor.id));
 await store.latest();expect(store.getSnapshot().page).toBe(1);expect(store.getSnapshot().records[0]?.id).toBe('record-2000');expect(store.getSnapshot().records).toHaveLength(20);
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
it('keeps incoming records out of visited pages until explicit refresh',async()=>{
 const f=fixture(Array.from({length:21},(_,n)=>record(n))),store=new HistoryFeedStore(f.port);await store.refresh();
 const original=store.getSnapshot().records.map(r=>r.id);f.speech.records.push(...Array.from({length:60},(_,n)=>record(100+n)));
 await store.refresh();expect(store.getSnapshot().records.map(r=>r.id)).toEqual(original);expect(store.getSnapshot().newRecords).toBe(true);
 await store.next();expect(store.getSnapshot().records.map(r=>r.id)).toEqual(['record-0']);
 await f.port.remove('record-0',1);store.remove('record-0');await waitFor(()=>expect(store.getSnapshot().page).toBe(1));
 expect(store.getSnapshot().records.map(r=>r.id)).toEqual(original);
 const retry=vi.spyOn(f.port,'status');await store.latest();expect(retry).toHaveBeenLastCalledWith(true);expect(store.getSnapshot().total).toBe(80);
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
 render(<MemoryRouter><History {...f} recordingSettings={recordingSettings(f.speech)} snapshot={initialSpeech()}/></MemoryRouter>);
 await screen.findByText('当前成功');fireEvent.click(screen.getByRole('button',{name:'搜索识别文字'}));fireEvent.change(screen.getByRole('textbox',{name:'搜索识别文字'}),{target:{value:'旧文字'}});
 await screen.findByText('匹配旧版本');const row=screen.getByRole('listitem');fireEvent.click(within(row).getByRole('button',{name:'复制'}));
 await waitFor(()=>expect(copy).toHaveBeenCalledWith(full));expect(within(row).getByRole('button',{name:'已复制'})).toBeVisible();
 const user=userEvent.setup();within(row).getByRole('button',{name:'更多操作'}).focus();await user.keyboard('{Enter}');await user.click(screen.getByRole('menuitem',{name:'识别版本'}));
 expect(await screen.findByRole('region',{name:'历史识别文字'})).toHaveTextContent(full);expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
 expect(screen.getByRole('combobox',{name:'识别版本'})).toHaveTextContent('首次识别');
});
it('keeps a successful body beside a failed retry and deletes only after inline confirmation',async()=>{
 const r=record();r.attempts.unshift({...r.attempts[0]!,id:'retry',kind:'retry',createdAt:9000,status:'failed',text:'',errorCode:'timeout'});const f=fixture([r]);
 render(<MemoryRouter><History {...f} recordingSettings={recordingSettings(f.speech)} snapshot={initialSpeech()}/></MemoryRouter>);await screen.findByText('完整的识别文字');expect(screen.getByText(/本次重识别未完成/)).toBeVisible();
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
 expect(store.getSnapshot().page).toBe(2);expect(store.getSnapshot().total).toBe(42);expect(store.getSnapshot().newRecords).toBe(true);expect(store.getSnapshot().records.some(r=>r.id==='record-100')).toBe(false);
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
 expect(store.getSnapshot()).toMatchObject({page:1,total:42,newRecords:true,error:'unavailable'});
 await store.retry();expect(store.getSnapshot().page).toBe(2);expect(store.getSnapshot().error).toBeNull();
});

it('seeks forward when the first page is emptied and shows empty only after the remaining pages disappear',async()=>{
 const f=fixture(Array.from({length:21},(_,n)=>record(n))),store=new HistoryFeedStore(f.port);await store.refresh();
 for(const row of store.getSnapshot().records){await f.port.remove(row.id,1);store.remove(row.id);}
 await store.refresh();await waitFor(()=>expect(store.getSnapshot().records.map(r=>r.id)).toEqual(['record-0']));
 expect(store.getSnapshot().page).toBe(2);await f.port.remove('record-0',1);store.remove('record-0');await store.refresh();
 expect(store.getSnapshot().records).toHaveLength(0);expect(store.getSnapshot().total).toBe(0);expect(store.getSnapshot().cursor).toBeNull();
});

it.each(['service','emptyText'])('shows translation error %s while retaining the recognized original',async errorCode=>{
 const r=record(1,'今天下午三点开会');r.attempts[0]!.purpose='translation';r.attempts[0]!.targetLanguage='en-US';
 r.processing=[{id:'failed-translation',attemptId:r.currentAttempt!,configName:'文字模型',modelId:'fixture',state:'failed',text:'',revision:1,errorCode,units:[]}];
 const f=fixture([r]),user=userEvent.setup();
 render(<MemoryRouter><History {...f} recordingSettings={recordingSettings(f.speech)} snapshot={initialSpeech()}/></MemoryRouter>);
 await screen.findByRole('listitem');screen.getByRole('button',{name:'更多操作'}).focus();await user.keyboard('{Enter}');await user.click(await screen.findByRole('menuitem',{name:'识别版本'}));
 const versions=await screen.findByRole('combobox',{name:'识别版本'});versions.focus();await user.keyboard('{Enter}');
 await user.click(await screen.findByRole('option',{name:'译文 · 文字模型'}));
 const body=screen.getByRole('region',{name:'历史识别文字'});expect(body).not.toHaveTextContent('未识别到文字');expect(body).toHaveTextContent(errorCode==='emptyText'?'未生成译文。':'翻译未完成。');
 versions.focus();await user.keyboard('{Enter}');await user.click(await screen.findByRole('option',{name:/原文/}));
 expect(body).toHaveTextContent('今天下午三点开会');
});
