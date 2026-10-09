import {describe,it,expect,vi} from 'vitest';
import {SettingStore} from '../src/settings/store';
const deferred=<T,>()=>{let resolve!:(value:T)=>void,reject!:(value:unknown)=>void;const promise=new Promise<T>((r,j)=>{resolve=r;reject=j;});return {promise,resolve,reject};};
const spin=async()=>{for(let i=0;i<8;i++)await Promise.resolve();};
describe('page-independent setting settlement',()=>{
 it('serializes fast reversal and finishes after the last subscriber leaves',async()=>{
  const first=deferred<{enabled:boolean;revision:number}>(),second=deferred<{enabled:boolean;revision:number}>();
  const save=vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  const store=new SettingStore(async()=>({enabled:true,revision:1}),save);await store.load();
  const release=store.subscribe(vi.fn());store.update({enabled:false});store.update({enabled:true});release();
  expect(save).toHaveBeenCalledTimes(1);first.resolve({enabled:false,revision:2});await spin();
  expect(save).toHaveBeenLastCalledWith({enabled:true,revision:2});expect(store.getSnapshot().value?.enabled).toBe(true);
  second.resolve({enabled:true,revision:3});await spin();expect(store.getSnapshot()).toMatchObject({busy:false,error:null,value:{enabled:true,revision:3}});
 });
 it('merges recording preferences against the last acknowledged microphone selection',async()=>{
  const first=deferred<{deviceId:string;retentionDays:number;revision:number}>();let current={deviceId:'old',retentionDays:30,revision:1};
  const save=vi.fn().mockImplementationOnce(()=>first.promise).mockImplementation(async value=>current={...value,revision:value.revision+1});
  const store=new SettingStore(async()=>current,save);await store.load();store.update({deviceId:'new'});store.update({retentionDays:7});
  current={deviceId:'new',retentionDays:30,revision:2};first.resolve(current);await spin();
  expect(save).toHaveBeenLastCalledWith({deviceId:'new',retentionDays:7,revision:2});expect(current).toEqual({deviceId:'new',retentionDays:7,revision:3});
 });
 it('rejects a stale read after a successful commit',async()=>{
  const stale=deferred<{enabled:boolean;revision:number}>();const read=vi.fn().mockResolvedValueOnce({enabled:true,revision:1}).mockReturnValueOnce(stale.promise);
  const store=new SettingStore(read,async()=>({enabled:false,revision:2}));await store.load();const loading=store.load();store.update({enabled:false});await spin();stale.resolve({enabled:true,revision:1});await loading;
  expect(store.getSnapshot()).toMatchObject({value:{enabled:false,revision:2},error:null});
 });
 it('recognizes a committed value after a lost reply without a duplicate write',async()=>{
  let current={enabled:true,revision:1};const save=vi.fn(async()=>{current={enabled:false,revision:2};throw Error('lost reply');});
  const store=new SettingStore(async()=>current,save);await store.load();store.update({enabled:false});await spin();
  expect(save).toHaveBeenCalledOnce();expect(store.getSnapshot()).toMatchObject({value:current,error:null,busy:false});
 });
 it('shows confirmed state on failure and does not automatically retry',async()=>{
  const save=vi.fn().mockRejectedValue({code:'unavailable'}),store=new SettingStore(async()=>({enabled:true,revision:1}),save);await store.load();store.update({enabled:false});await spin();
  expect(store.getSnapshot()).toMatchObject({value:{enabled:true,revision:1},error:'保存失败，请重试。',busy:false});expect(save).toHaveBeenCalledOnce();
 });
 it('does not pretend to roll back an unknown write and reconciles on explicit reload',async()=>{
  const read=vi.fn().mockResolvedValueOnce({enabled:true,revision:1}).mockRejectedValueOnce(Error()).mockResolvedValueOnce({enabled:false,revision:2});
  const save=vi.fn().mockRejectedValue(Error()),store=new SettingStore(read,save);await store.load();store.update({enabled:false});await spin();
  expect(store.getSnapshot()).toMatchObject({ready:false,value:{enabled:false},error:'暂时无法确认设置，请重新读取。'});
  await store.load();expect(store.getSnapshot()).toMatchObject({ready:true,value:{enabled:false,revision:2},error:null});expect(save).toHaveBeenCalledOnce();
 });
 it('recovers an unavailable initial read without accepting an uninitialized operation',async()=>{
  const save=vi.fn(),read=vi.fn().mockRejectedValueOnce(Error()).mockResolvedValueOnce({enabled:true,revision:1});const store=new SettingStore(read,save);
  await store.load();store.update({enabled:false});expect(save).not.toHaveBeenCalled();expect(store.getSnapshot().error).toBe('无法读取设置，请重试。');await store.load();expect(store.getSnapshot().ready).toBe(true);
 });
 it('retains the newer queued intent while an earlier write is unknown',async()=>{
  const first=deferred<{enabled:boolean;revision:number}>();
  const read=vi.fn().mockResolvedValueOnce({enabled:true,revision:1}).mockRejectedValueOnce(Error()).mockResolvedValueOnce({enabled:false,revision:2});
  const save=vi.fn().mockReturnValueOnce(first.promise).mockResolvedValueOnce({enabled:true,revision:3});const store=new SettingStore(read,save);
  await store.load();store.update({enabled:false});store.update({enabled:true});first.reject(Error('reply lost'));await spin();
  expect(store.getSnapshot()).toMatchObject({ready:false,value:{enabled:true}});expect(save).toHaveBeenCalledOnce();
  await store.load();await spin();expect(save).toHaveBeenLastCalledWith({enabled:true,revision:2});expect(save).toHaveBeenCalledTimes(2);expect(store.getSnapshot()).toMatchObject({value:{enabled:true,revision:3},error:null});
 });
 it('settles a queued reversal already matching readback without reporting its superseded failure',async()=>{
  const first=deferred<{enabled:boolean;revision:number}>(),save=vi.fn().mockReturnValue(first.promise);
  const store=new SettingStore(async()=>({enabled:true,revision:1}),save);await store.load();store.update({enabled:false});store.update({enabled:true});first.reject(Error());await spin();
  expect(save).toHaveBeenCalledOnce();expect(store.getSnapshot()).toMatchObject({value:{enabled:true},busy:false,error:null});
 });
});
