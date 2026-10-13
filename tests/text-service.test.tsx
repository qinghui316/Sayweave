import {act,render,screen,waitFor} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {describe,expect,it,vi} from 'vitest';
import {TextServiceSettings} from '../src/features/settings/TextServiceSettings';
import {parseTextService,type TextServicePort,type TextServiceStatus} from '../src/contracts/text-service';
const ready:TextServiceStatus={installed:true,enabled:true,updateAvailable:false,payloadAvailable:true,cleanupPending:false,deploymentState:'ready',error:null};
function service(value:TextServiceStatus):TextServicePort{return {mode:'native',status:vi.fn().mockResolvedValue(value),act:vi.fn().mockResolvedValue(ready)};}
describe('input recovery',()=>{
 it('keeps healthy input invisible without maintenance or enable controls',async()=>{
  const port=service(ready),view=render(<TextServiceSettings port={port}/>);
  await act(async()=>{});
  expect(view.container).toBeEmptyDOMElement();expect(port.act).not.toHaveBeenCalled();
 });
 it('does not expose retired file cleanup as a user setting',async()=>{
  const port=service({...ready,cleanupPending:true}),view=render(<TextServiceSettings port={port}/>);
  await act(async()=>{});expect(view.container).toBeEmptyDOMElement();
 });
 it('does not interrupt automatic installer work with another operation',async()=>{
  const port=service({...ready,updateAvailable:true,deploymentState:'pending'}),view=render(<TextServiceSettings port={port}/>);
  await act(async()=>{});expect(view.container).toBeEmptyDOMElement();expect(port.act).not.toHaveBeenCalled();
 });
 it('requires explicit recovery before restoring a previously disabled choice',async()=>{
  const port=service({...ready,enabled:false}),user=userEvent.setup(),view=render(<TextServiceSettings port={port}/>);
  const button=await screen.findByRole('button',{name:'恢复输入'});
  expect(port.act).not.toHaveBeenCalled();expect(screen.queryByRole('switch')).not.toBeInTheDocument();
  await user.click(button);expect(port.act).toHaveBeenCalledExactlyOnceWith('enable');
  await waitFor(()=>expect(view.container).toBeEmptyDOMElement());
 });
 it('repairs a damaged owned registration and preserves native restart instructions',async()=>{
  const port=service({...ready,enabled:false,updateAvailable:true,error:'componentRegistration'});
  vi.mocked(port.act).mockResolvedValue({...ready,restartRequired:true});
  render(<TextServiceSettings port={port}/>);await userEvent.click(await screen.findByRole('button',{name:'恢复输入'}));
  expect(port.act).toHaveBeenCalledExactlyOnceWith('install');
  await screen.findByText('请重新打开正在使用的输入应用一次。');
  expect(screen.queryByRole('button',{name:'恢复输入'})).not.toBeInTheDocument();
 });
 it('only offers failed transaction repair when the native owner allows it',async()=>{
  const port=service({...ready,deploymentState:'failed'}),view=render(<TextServiceSettings port={port}/>);
  await screen.findByText('输入组件维护未完成，请重新运行安装包恢复。');
  expect(screen.queryByRole('button')).not.toBeInTheDocument();
  view.rerender(<TextServiceSettings port={service({...ready,deploymentState:'failed',maintenanceAvailable:true})}/>);
  await screen.findByRole('button',{name:'恢复输入'});
 });
 it('shows failed recovery accurately and ignores replies after unmount',async()=>{
  const port=service({...ready,enabled:false}),user=userEvent.setup();
  let finish!:(v:TextServiceStatus)=>void;
  vi.mocked(port.act).mockRejectedValueOnce({code:'componentEnableFailed'}).mockImplementationOnce(()=>new Promise(r=>{finish=r;}));
  const view=render(<TextServiceSettings port={port}/>);
  await user.click(await screen.findByRole('button',{name:'恢复输入'}));await screen.findByText('未能启用输入组件，请重试。');
  await user.click(screen.getByRole('button',{name:'恢复输入'}));
  expect(screen.queryByRole('button')).not.toBeInTheDocument();view.unmount();
  await act(async()=>{finish(ready);});expect(port.act).toHaveBeenCalledTimes(2);
 });
 it('can retry a failed status read without performing a system mutation',async()=>{
  const port=service(ready);vi.mocked(port.status).mockRejectedValueOnce(new Error('unavailable'));
  const view=render(<TextServiceSettings port={port}/>);
  await userEvent.click(await screen.findByRole('button',{name:'重新检查'}));
  await waitFor(()=>expect(view.container).toBeEmptyDOMElement());expect(port.act).not.toHaveBeenCalled();
 });
 it('rejects impossible or malformed native status',()=>{
  for(const invalid of [{...ready,installed:false},{...ready,updateAvailable:true,payloadAvailable:false},{...ready,restartRequired:'true'},{...ready,maintenanceAvailable:1}])expect(()=>parseTextService(invalid)).toThrow();
 });
});
