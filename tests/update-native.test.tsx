import {act,render,screen,cleanup} from '@testing-library/react';
import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {useSyncExternalStore} from 'react';
import {initialUpdate,type UpdateSnapshot} from '../src/contracts/update';
const mock=vi.hoisted(()=>({invoke:vi.fn(),listen:vi.fn(),release:vi.fn(),events:[] as ((event:{payload:UpdateSnapshot})=>void)[]}));
vi.mock('@tauri-apps/api/core',()=>({isTauri:()=>true,invoke:mock.invoke}));
vi.mock('@tauri-apps/api/event',()=>({listen:mock.listen}));
import {NativeUpdate} from '../src/adapters/update-tauri';
function View({port}:{port:NativeUpdate}){const value=useSyncExternalStore(port.subscribe,port.snapshot);return <span>{value.build.version} {value.state}</span>;}
beforeEach(()=>{mock.events=[];mock.invoke.mockReset();mock.listen.mockReset();mock.release.mockReset();mock.listen.mockImplementation(async(_name,listener)=>{mock.events.push(listener);return mock.release;});mock.invoke.mockResolvedValue({...initialUpdate,revision:1,build:{...initialUpdate.build,version:'0.1.7'}});});
afterEach(cleanup);
describe('real native adapter mounted by React',()=>{
 it('binds methods and rejects a stale snapshot after an event',async()=>{
  const port=new NativeUpdate();render(<View port={port}/>);expect(await screen.findByText('0.1.7 idle')).toBeInTheDocument();
  act(()=>mock.events[0]!({payload:{...port.snapshot(),revision:4,state:'readyToInstall',taskId:'candidate'}}));
  expect(screen.getByText('0.1.7 readyToInstall')).toBeInTheDocument();
  act(()=>mock.events[0]!({payload:{...port.snapshot(),revision:2,state:'failed'}}));
  expect(screen.getByText('0.1.7 readyToInstall')).toBeInTheDocument();
  cleanup();expect(mock.release).toHaveBeenCalledOnce();expect(mock.invoke).not.toHaveBeenCalledWith('update_cancel');
 });
 it('installation carries the exact task and revision and leaves execution native',async()=>{
  const port=new NativeUpdate();await port.install('candidate-a',42);
  expect(mock.invoke).toHaveBeenCalledWith('update_install',{taskId:'candidate-a',revision:42});
 });
 it('retires a subscription that completes after unmount',async()=>{
  let resolve!:(release:()=>void)=>void;mock.listen.mockReturnValue(new Promise<()=>void>(r=>{resolve=r;}));
  const port=new NativeUpdate(),stop=port.subscribe(()=>{});stop();await act(async()=>resolve(mock.release));
  expect(mock.release).toHaveBeenCalledOnce();expect(mock.invoke).not.toHaveBeenCalled();
 });
});
