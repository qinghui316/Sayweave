import {act,cleanup,fireEvent,render,screen} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {afterEach,expect,it,vi} from 'vitest';
import {initialUpdate,type UpdatePort,type UpdateSnapshot} from '../src/contracts/update';
import {UpdateDock} from '../src/ui/UpdateDock';

afterEach(cleanup);
function fixture(patch:Partial<UpdateSnapshot>={}) {
 let value:UpdateSnapshot={...initialUpdate,...patch};const listeners=new Set<()=>void>();
 const port:UpdatePort={snapshot:()=>value,subscribe:fn=>{listeners.add(fn);return()=>{listeners.delete(fn);};},check:vi.fn(),cancel:vi.fn(),install:vi.fn(),restart:vi.fn()};
 return {port,update:(next:Partial<UpdateSnapshot>)=>{value={...value,...next};listeners.forEach(fn=>fn());}};
}
it.each(['idle','checking','failed'] as const)('shows About without an offer in %s',async state=>{
 const {port}=fixture({state}),about=vi.fn();const user=userEvent.setup();
 render(<UpdateDock port={port} onAbout={about} onInstall={vi.fn()} preparing={false} error={null} dismissKey="home"/>);
 await user.click(screen.getByRole('button',{name:'关于'}));expect(about).toHaveBeenCalledOnce();expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
});
it('updates progress in the same slot, dismisses without cancelling, and returns keyboard focus',async()=>{
 const f=fixture(),user=userEvent.setup();render(<UpdateDock port={f.port} onAbout={vi.fn()} onInstall={vi.fn()} preparing={false} error={null} dismissKey="home"/>);
 act(()=>f.update({state:'downloading',version:'0.1.9',progress:24,taskId:'one',build:{...initialUpdate.build,updatesEnabled:true}}));
 const trigger=screen.getByRole('button',{name:'正在下载更新 24%'});await user.click(trigger);expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow','24');
 act(()=>f.update({progress:68}));expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow','68');
 fireEvent.keyDown(document,{key:'Escape'});expect(screen.queryByRole('dialog')).not.toBeInTheDocument();expect(trigger).toHaveFocus();expect(f.port.cancel).not.toHaveBeenCalled();
});
it('keeps install admission outside the popover and closes when its host changes',async()=>{
 const f=fixture({state:'readyToInstall',version:'0.1.9',taskId:'one',build:{...initialUpdate.build,updatesEnabled:true}}),install=vi.fn(),user=userEvent.setup();
 const props={port:f.port,onAbout:vi.fn(),onInstall:install,preparing:false,error:null};const view=render(<UpdateDock {...props} dismissKey="home"/>);
 await user.click(screen.getByRole('button',{name:'重启并更新到 0.1.9'}));await user.click(screen.getByRole('button',{name:'重启并更新'}));expect(install).toHaveBeenCalledOnce();expect(f.port.install).not.toHaveBeenCalled();
 view.rerender(<UpdateDock {...props} dismissKey="settings"/>);expect(screen.queryByRole('dialog')).not.toBeInTheDocument();expect(f.port.cancel).not.toHaveBeenCalled();
});
