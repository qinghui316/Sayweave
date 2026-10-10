import {cleanup,render,screen} from '@testing-library/react';
import {afterEach,expect,it,vi} from 'vitest';
import {initialUpdate,type UpdatePort,type UpdateState} from '../src/contracts/update';
import {UpdatePanel} from '../src/ui/UpdatePanel';
afterEach(cleanup);
function port(state:UpdateState):UpdatePort {
 const snapshot={...initialUpdate,state,version:'0.1.8',taskId:'package',build:{...initialUpdate.build,version:'0.1.7',updatesEnabled:true},progress:state==='downloading'?43:null};
 return{snapshot:()=>snapshot,subscribe:()=>()=>{},check:vi.fn(),cancel:vi.fn(),install:vi.fn(),restart:vi.fn()};
}
it.each([['preparing','正在准备更新'],['stopping','正在停止服务'],['installing','正在安装']] as const)('renders the native %s phase even while the renderer waits for install', (state,label)=>{
 render(<UpdatePanel port={port(state)} onInstall={vi.fn()} preparing error={null}/>);
 expect(screen.getByRole('button',{name:label})).toBeDisabled();
 expect(screen.queryByRole('button',{name:'检查更新'})).not.toBeInTheDocument();
 expect(screen.getByRole('status')).not.toHaveTextContent('正在保存设置');
});
it('shows actual download percentage and allows cancellation',()=>{
 render(<UpdatePanel port={port('downloading')} onInstall={vi.fn()} preparing={false} error={null}/>);
 expect(screen.getByRole('progressbar')).toHaveAttribute('aria-valuenow','43');
 expect(screen.getByRole('button',{name:'取消'})).toBeEnabled();
 expect(screen.queryByRole('button',{name:'重启并更新'})).not.toBeInTheDocument();
});
