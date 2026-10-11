import {act,render,screen,waitFor} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {describe,it,expect,vi} from 'vitest';
import {BrowserLanguage} from '../src/adapters/language-tauri';
import {parseProfileOverview,parseProfileDetails,type ProfileOverview,type ProfileDetails} from '../src/contracts/language';
import {PersonalizationDashboard} from '../src/features/preferences/PersonalizationDashboard';
import {PersonalizationSettings} from '../src/features/preferences/PersonalizationSettings';
import {SettingStore} from '../src/settings/store';
const a='app:'+'a'.repeat(64),b='app:'+'b'.repeat(64);
const summary=(scope:string,name:string,version=1)=>({scope,name,version,preferenceCount:1,observationCount:2,updatedAt:1000});
const overview:ProfileOverview={preferenceCount:3,applicationCount:2,observationCount:4,shared:summary('shared','通用习惯'),applications:[summary(a,'Codex'),summary(b,'微信')],next:null};
const profile=(scope:string,rule:string,version=1):ProfileDetails=>({scope,version,updatedAt:1000,preferences:[{rule,language:'any',context:null}]});
const deferred=<T,>()=>{let resolve!:(value:T)=>void;return {promise:new Promise<T>(r=>{resolve=r;}),resolve:(value:T)=>resolve(value)};};
describe('personalization read projections',()=>{
 it('validates scope/version/counts and projects only permitted fields',()=>{
  expect(parseProfileOverview(overview)).toEqual(overview);
  expect(()=>parseProfileOverview({...overview,observationCount:-1})).toThrow();
  expect(()=>parseProfileOverview({...overview,applications:[overview.shared]})).toThrow();
  expect(parseProfileDetails({...profile(a,'保持自然语气'),analysis:'secret'})).not.toHaveProperty('analysis');
  expect(()=>parseProfileDetails({...profile(a,'rule'),scope:'bad'})).toThrow();
 });
 it('shows actual zero without generating a fake progress measure',async()=>{
  render(<PersonalizationDashboard port={new BrowserLanguage()} visible/>);await screen.findByText('使用一段时间后，这里会显示学到的表达习惯。');
  expect(screen.getByText('已学表达习惯')).toBeVisible();expect(screen.queryByText(/%/)).not.toBeInTheDocument();
 });
 it('keeps shared habits separate and drops details belonging to a previous application',async()=>{
  const port=new BrowserLanguage(),late=deferred<ProfileDetails>();vi.spyOn(port,'overview').mockResolvedValue(overview);
  vi.spyOn(port,'profile').mockImplementation(scope=>scope===a?late.promise:Promise.resolve(profile(scope,'微信保持短段落')));
  const user=userEvent.setup();render(<PersonalizationDashboard port={port} visible/>);await screen.findByRole('button',{name:/通用习惯/});
  await user.click(screen.getByRole('button',{name:/Codex/}));await user.click(screen.getByRole('button',{name:/微信/}));await screen.findByText('微信保持短段落');
  await act(async()=>late.resolve(profile(a,'旧应用结果')));expect(screen.queryByText('旧应用结果')).not.toBeInTheDocument();
 });
 it('ignores old profile versions after a refreshed overview',async()=>{
  const port=new BrowserLanguage(),late=deferred<ProfileDetails>();vi.spyOn(port,'overview').mockResolvedValue(overview);
  const read=vi.spyOn(port,'profile').mockReturnValueOnce(late.promise).mockResolvedValue(profile(a,'新版本习惯',2));
  const user=userEvent.setup(),view=render(<PersonalizationDashboard port={port} visible/>);await screen.findByRole('button',{name:/Codex/});await user.click(screen.getByRole('button',{name:/Codex/}));
  vi.mocked(port.overview).mockResolvedValue({...overview,applications:[summary(a,'Codex',2),overview.applications[1]!]});
  view.rerender(<PersonalizationDashboard port={port} visible={false}/>);view.rerender(<PersonalizationDashboard port={port} visible/>);
  await screen.findByText('新版本习惯');await act(async()=>late.resolve(profile(a,'旧版本习惯')));expect(screen.queryByText('旧版本习惯')).not.toBeInTheDocument();expect(read.mock.calls.every(([scope])=>scope===a)).toBe(true);
 });
 it('keeps preference saving available when the dashboard query fails',async()=>{
  const port=new BrowserLanguage();vi.spyOn(port,'overview').mockRejectedValue(Error());const store=new SettingStore(async()=>(await port.settings()).personalization,async personalization=>(await port.save({personalization})).personalization);
  const user=userEvent.setup();render(<PersonalizationSettings store={store} port={port} visible/>);await screen.findByText('暂时无法读取学习数据，请重试。');const toggle=screen.getByRole('switch',{name:'个性化'});await waitFor(()=>expect(toggle).toBeEnabled());await user.click(toggle);await waitFor(async()=>expect((await port.settings()).personalization.enabled).toBe(false));
 });
 it('drops details from a page that has been left',async()=>{
  const port=new BrowserLanguage(),late=deferred<ProfileDetails>();
  vi.spyOn(port,'overview').mockResolvedValueOnce({...overview,applications:[overview.applications[0]!],next:a}).mockResolvedValue({...overview,applications:[overview.applications[1]!],next:null});
  vi.spyOn(port,'profile').mockReturnValue(late.promise);const user=userEvent.setup();render(<PersonalizationDashboard port={port} visible/>);await screen.findByRole('button',{name:/Codex/});await user.click(screen.getByRole('button',{name:/Codex/}));await user.click(screen.getByRole('button',{name:'下一页'}));await screen.findByRole('button',{name:/微信/});
  await act(async()=>late.resolve(profile(a,'已离开页面的资料')));expect(screen.queryByText('已离开页面的资料')).not.toBeInTheDocument();expect(screen.queryByRole('button',{name:/Codex/})).not.toBeInTheDocument();
 });
 it('refreshes on window focus and releases its listener on unmount',async()=>{
  const port=new BrowserLanguage(),read=vi.spyOn(port,'overview');const view=render(<PersonalizationDashboard port={port} visible/>);await screen.findByText('使用一段时间后，这里会显示学到的表达习惯。');
  const before=read.mock.calls.length;await act(async()=>{window.dispatchEvent(new Event('focus'));await new Promise(resolve=>setTimeout(resolve,180));});expect(read.mock.calls.length).toBeGreaterThan(before);view.unmount();const after=read.mock.calls.length;
  await act(async()=>{window.dispatchEvent(new Event('focus'));await new Promise(resolve=>setTimeout(resolve,180));});expect(read.mock.calls).toHaveLength(after);
 });
});
