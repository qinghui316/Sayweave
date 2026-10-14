import {act,cleanup,render,screen} from '@testing-library/react';
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
it.each(['idle','checking','downloading','verifying','readyToInstall','preparing','stopping','installing','failed','recovery'] as const)('routes %s to About without starting installation',async state=>{
 const f=fixture({state,version:state==='idle'?null:'0.1.11'}),about=vi.fn(),user=userEvent.setup();
 render(<UpdateDock port={f.port} onAbout={about}/>);
 await user.tab();expect(screen.getByRole('button')).toHaveFocus();await user.keyboard('{Enter}');
 expect(about).toHaveBeenCalledOnce();expect(screen.queryByRole('dialog')).not.toBeInTheDocument();expect(f.port.install).not.toHaveBeenCalled();expect(f.port.check).not.toHaveBeenCalled();
});
it('uses actual progress and waits for verification before showing download readiness',()=>{
 const f=fixture({state:'downloading',version:'0.1.11',progress:null});const view=render(<UpdateDock port={f.port} onAbout={vi.fn()}/>);
 const visual=()=>view.container.querySelector('[data-update-visual]');
 expect(visual()).toHaveAttribute('data-update-visual','indeterminate');
 for(const progress of [0,43,100]){act(()=>f.update({progress}));expect(visual()).toHaveAttribute('data-update-visual','progress');expect(visual()?.querySelector('[stroke-dashoffset]')).toHaveAttribute('stroke-dashoffset',String(100-progress));}
 act(()=>f.update({state:'verifying'}));expect(visual()).toHaveAttribute('data-update-visual','indeterminate');
 act(()=>f.update({state:'readyToInstall'}));expect(visual()).toHaveAttribute('data-update-visual','download');
 act(()=>f.update({state:'failed'}));expect(visual()).toHaveAttribute('data-update-visual','error');
});
