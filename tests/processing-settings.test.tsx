import {act,render,screen,waitFor} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {describe,it,expect,vi} from 'vitest';
import {ProcessingSettings} from '../src/features/settings';
import type {ProcessingPort} from '../src/contracts/processing';
import {SettingStore} from '../src/settings/store';
import {ToggleSetting} from '../src/settings/ToggleSetting';
import {BrowserLanguage} from '../src/adapters/language-tauri';
const initial={enabled:true,revision:1};
function fixture(){
 let value={...initial};
 const port:ProcessingPort={settings:vi.fn(async()=>({...value})),status:vi.fn(async()=>({settings:{...value},available:true,model:'文字服务',error:null})),save:vi.fn(async next=>value={...next,revision:next.revision+1})};
 const store=new SettingStore(()=>port.settings(),next=>port.save(next));return {port,store};
}
describe('immediate feature settings',()=>{
 it('shows only cleanup and persists without a second action',async()=>{
  const {port,store}=fixture(),user=userEvent.setup();render(<ProcessingSettings port={port} store={store} revision={0}/>);
  const toggle=screen.getByRole('switch',{name:'整理口述'});await waitFor(()=>expect(toggle).toBeEnabled());await user.click(toggle);
  await waitFor(()=>expect(port.save).toHaveBeenCalledWith({enabled:false,revision:1}));expect(screen.getAllByRole('switch')).toHaveLength(1);expect(screen.queryByRole('button',{name:'保存设置'})).not.toBeInTheDocument();expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
 });
 it('does not depend on model metadata and finishes a save after the page closes',async()=>{
  const {port,store}=fixture();let resolve!:(value:typeof initial)=>void;vi.mocked(port.status).mockRejectedValue(Error('model metadata'));vi.mocked(port.save).mockReturnValue(new Promise(r=>{resolve=r;}));
  const user=userEvent.setup(),view=render(<ProcessingSettings port={port} store={store} revision={0}/>);const toggle=screen.getByRole('switch');await waitFor(()=>expect(toggle).toBeEnabled());await user.click(toggle);view.unmount();
  await act(async()=>{resolve({enabled:false,revision:2});});expect(store.getSnapshot()).toMatchObject({value:{enabled:false,revision:2},busy:false,error:null});
 });
 it('keeps an unavailable control visible and can retry its read',async()=>{
  const {port,store}=fixture();vi.mocked(port.settings).mockRejectedValueOnce(Error());const user=userEvent.setup();render(<ProcessingSettings port={port} store={store} revision={0}/>);
  await screen.findByText('无法读取设置，请重试。');expect(screen.getByRole('switch')).toBeDisabled();await user.click(screen.getByRole('button',{name:'重新读取'}));await waitFor(()=>expect(screen.getByRole('switch')).toBeEnabled());
 });
 it('saves personalization independently without disabling automatic terms',async()=>{
  const port=new BrowserLanguage(),store=new SettingStore(async()=>(await port.settings()).personalization,async personalization=>(await port.save({personalization})).personalization);const user=userEvent.setup();
  render(<ToggleSetting store={store} label="个性化" description="按应用学习表达习惯。"/>);await waitFor(()=>expect(screen.getByRole('switch')).toBeEnabled());await user.click(screen.getByRole('switch'));
  await waitFor(async()=>expect((await port.settings()).personalization).toEqual({enabled:false,revision:2}));expect((await port.settings()).automaticTerms).toEqual(initial);
 });
});
