import {act,fireEvent,render,screen,waitFor} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {expect,it,vi} from 'vitest';
import {BrowserTranslation} from '../src/adapters/translation-tauri';
import {BrowserShortcuts} from '../src/adapters/dictation-tauri';
import {parseTranslation} from '../src/contracts/translation';
import {initialDictation} from '../src/contracts/dictation';
import {TranslationSettings} from '../src/features/settings/TranslationSettings';
import {ShortcutSettings} from '../src/features/settings/ShortcutSettings';
import {TranslationTarget} from '../src/features/voice/TranslationTarget';
import {VoiceSurface} from '../src/features/voice';
import {SettingStore} from '../src/settings/store';
import {shortcutSettings} from './settings-fixtures';

it('accepts the complete language catalog and rejects missing, duplicate and unknown targets',async()=>{
 const config=await new BrowserTranslation().read();
 expect(config.languages).toHaveLength(22);
 expect(config.settings.targets).toEqual(['en-US','ja','ko','fr','zh-Hans']);
 for(const targets of [[],['en-US','en-US'],['invalid']])expect(()=>parseTranslation({...config,settings:{...config.settings,targets}})).toThrow();
 const targets=config.languages.map(l=>l.code);
 expect(parseTranslation({...config,settings:{...config.settings,targets}}).settings.targets).toEqual(targets);
 expect(()=>parseTranslation({...config,settings:{...config.settings,activeTarget:'unknown'}})).toThrow();
});
it('preserves saved order without reorder controls, replaces a removed target and rejects stale saves',async()=>{
 const port=new BrowserTranslation(),initial=await port.read();
 const changed=await port.save({...initial.settings,targets:['ja','en-US'],activeTarget:'ja'});
 await expect(port.save(initial.settings)).rejects.toMatchObject({code:'conflict'});
 const store=new SettingStore(async()=>(await port.read()).settings,async v=>(await port.save(v)).settings),user=userEvent.setup();
 render(<TranslationSettings port={port} store={store}/>);
 await screen.findByRole('button',{name:'移除英语（美国）'});
 expect(screen.queryByRole('button',{name:/上移|下移/})).not.toBeInTheDocument();
 expect(store.getConfirmed()?.targets).toEqual(['ja','en-US']);
 await user.click(screen.getByRole('button',{name:'移除日语'}));
 await waitFor(()=>expect(store.getConfirmed()?.activeTarget).toBe('en-US'));
 expect(screen.getByRole('button',{name:'移除英语（美国）'})).toBeDisabled();
 expect((await port.read()).settings.revision).toBe(changed.settings.revision+1);
});
it('adds every remaining language beyond five, reloads, then removes and re-adds one',async()=>{
 const port=new BrowserTranslation(),config=await port.read(),user=userEvent.setup();
 const store=new SettingStore(async()=>(await port.read()).settings,async v=>(await port.save(v)).settings);
 render(<TranslationSettings port={port} store={store}/>);
 const add=await screen.findByRole('combobox',{name:'添加翻译语言'});
 const expected=[...config.settings.targets];
 for(const language of config.languages.filter(l=>!expected.includes(l.code))){
  await waitFor(()=>expect(add).toBeEnabled());add.focus();await user.keyboard('{Enter}');
  await user.click(await screen.findByRole('option',{name:language.label}));expected.push(language.code);
  await waitFor(()=>expect(store.getConfirmed()?.targets).toEqual(expected));
 }
 expect(add).toBeDisabled();await act(()=>store.load());
 expect(store.getConfirmed()?.targets).toEqual(expected);
 await user.click(screen.getByRole('button',{name:'移除日语'}));
 await waitFor(()=>expect(add).toBeEnabled());add.focus();await user.keyboard('{Enter}');
 await user.click(await screen.findByRole('option',{name:'日语'}));
 await waitFor(()=>expect(store.getConfirmed()?.targets).toEqual([...expected.filter(c=>c!=='ja'),'ja']));
},20000); // Seventeen menu/save cycles share CPU with the full suite; each assertion remains bounded.
it('captures a translation key without overwriting the ordinary shortcut',async()=>{
 const port=new BrowserShortcuts(),store=shortcutSettings(port),user=userEvent.setup();
 render(<ShortcutSettings port={port} settings={store}/>);
 await screen.findByText('已启用：右 Alt');
 await user.click(screen.getByRole('button',{name:'更改翻译快捷键'}));
 fireEvent.keyDown(window,{key:'F9',code:'F9'});
 await waitFor(()=>expect(store.getConfirmed()?.translationBinding).toBe('F9'));
 expect(store.getConfirmed()?.binding).toBe('AltRight');
});
it('binds selection to the recording task and keeps authoritative selection after rejection',async()=>{
 const base=new BrowserTranslation(),read=await base.read();
 await base.save({...read.settings,targets:['en-US','ja']});
 const port={read:()=>base.read(),save:base.save.bind(base),menu:vi.fn().mockResolvedValue(undefined),select:vi.fn().mockRejectedValue({code:'staleSession'})};
 const snapshot={...initialDictation(),purpose:'translation' as const,taskId:'A',targetLanguage:'en-US',translationTargets:['en-US','ja'],translationSelectable:true,translationMenu:true};
 const view=render(<TranslationTarget snapshot={snapshot} port={port}/>),user=userEvent.setup();
 await screen.findByRole('menuitemradio',{name:'日语'});
 await user.click(screen.getByRole('menuitemradio',{name:'日语'}));
 await screen.findByText('未切换，请重试');
 expect(port.select).toHaveBeenCalledExactlyOnceWith('A','ja');
 expect(screen.getByRole('button',{name:'翻译目标语言'})).toHaveTextContent('英语（美国）');
 view.rerender(<TranslationTarget snapshot={{...snapshot,translationSelectable:false,translationMenu:false}} port={port}/>);
 await waitFor(()=>expect(screen.queryByRole('menuitemradio')).not.toBeInTheDocument());
 expect(screen.getByRole('button',{name:'翻译目标语言'})).toBeDisabled();
});
it('shows translation processing without publishing a recording transcript preview',async()=>{
 const snapshot={...initialDictation(),purpose:'translation' as const,taskId:'A',targetLanguage:'en-US',translationTargets:['en-US'],presentation:'recognizing' as const};
 await act(async()=>{render(<VoiceSurface phase="processing" presentation="recognizing" text="私有识别正文" translation={snapshot} translationPort={new BrowserTranslation()} start={()=>{}} finish={()=>{}} cancel={()=>{}}/>);});
 expect(screen.getByRole('group',{name:'翻译中'})).toBeInTheDocument();
 expect(screen.queryByText('私有识别正文')).not.toBeInTheDocument();
});

it('reloads the language catalog after a failed read without leaving settings',async()=>{
 const base=new BrowserTranslation(),config=await base.read();
 const port={read:vi.fn().mockRejectedValueOnce(new Error('offline')).mockResolvedValue(config),save:base.save.bind(base),menu:base.menu.bind(base),select:base.select.bind(base)};
 const store=new SettingStore(async()=>config.settings,async v=>(await base.save(v)).settings),user=userEvent.setup();
 render(<TranslationSettings port={port} store={store}/>);
 await screen.findByText('无法读取语言，请重试。');
 expect(screen.getByRole('combobox',{name:'添加翻译语言'})).toBeDisabled();
 await user.click(screen.getByRole('button',{name:'重新读取'}));
 await user.click(screen.getByRole('button',{name:'移除中文（简体）'}));
 await waitFor(()=>expect(screen.getByRole('combobox',{name:'添加翻译语言'})).toBeEnabled());
 expect(screen.queryByText('无法读取语言，请重试。')).not.toBeInTheDocument();
 screen.getByRole('combobox',{name:'添加翻译语言'}).focus();await user.keyboard('{Enter}');
 await user.click(await screen.findByRole('option',{name:'德语'}));
 await waitFor(()=>expect(store.getConfirmed()?.targets).toEqual(['en-US','ja','ko','fr','de']));
});
