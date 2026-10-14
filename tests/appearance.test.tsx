import {act,render,screen,fireEvent} from '@testing-library/react';
import {it,expect,vi} from 'vitest';
import {themes,themeById,applyTheme} from '../src/design/themes';
import {AppearanceSettings} from '../src/features/settings/AppearanceSettings';
import {SettingStore} from '../src/settings/store';
import {parseAppearance,type Appearance} from '../src/contracts/appearance';

const resolve=(vars:Record<string,string>,key:string):string=>{const value=vars[key]!;return value.startsWith('var(')?resolve(vars,value.slice(4,-1)):value;};
const luminance=(hex:string)=>{const rgb=hex.slice(1).match(/../g)!.map(v=>parseInt(v,16)/255).map(v=>v<=.04045?v/12.92:((v+.055)/1.055)**2.4);return rgb[0]!*.2126+rgb[1]!*.7152+rgb[2]!*.0722;};
const contrast=(a:string,b:string)=>{const x=luminance(a),y=luminance(b);return (Math.max(x,y)+.05)/(Math.min(x,y)+.05);};

it('presets share complete color roles, resolve startup colors and preserve readable graphite controls',()=>{
 expect(new Set(themes.map(t=>t.id)).size).toBe(themes.length);
 for(const theme of themes){
  expect(Object.keys(theme.vars).sort()).toEqual(Object.keys(themes[0]!.vars).sort());
  expect(resolve(theme.vars,'--surface-canvas')).toMatch(/^#[0-9a-f]{6}$/i);
  expect(contrast(resolve(theme.vars,'--action-bg'),resolve(theme.vars,'--action-fg'))).toBeGreaterThanOrEqual(4.5);
  expect(contrast(resolve(theme.vars,'--voice-bg'),resolve(theme.vars,'--voice-fg'))).toBeGreaterThanOrEqual(4.5);
  expect(contrast(resolve(theme.vars,'--signal'),resolve(theme.vars,'--on-signal'))).toBeGreaterThanOrEqual(4.5);
 }
 const dark=themeById('graphite').vars;
 for(const surface of ['--surface-content','--surface-canvas','--surface-key','--surface-work','--surface-hover']){
  for(const text of ['--text-primary','--text-secondary'])expect(contrast(resolve(dark,surface),resolve(dark,text)),`${text} on ${surface}`).toBeGreaterThanOrEqual(4.5);
 }
 expect(contrast(dark['--voice-bg'],dark['--voice-button'])).toBeGreaterThanOrEqual(3);
 expect(contrast(dark['--action-bg'],dark['--selected-icon'])).toBeGreaterThanOrEqual(3);
 expect(contrast(dark['--switch-track'],dark['--switch-thumb'])).toBeGreaterThanOrEqual(3);
 expect(contrast(dark['--surface-key'],dark['--switch-off-thumb'])).toBeGreaterThanOrEqual(3);
 expect(contrast('#e1e1e1',themeById('light').vars['--switch-off-outline'])).toBeGreaterThanOrEqual(3);
});
it('unknown theme restores light variables without changing geometry',()=>{
 document.documentElement.style.setProperty('--radius-control','17px');
 applyTheme('graphite');expect(document.documentElement.style.getPropertyValue('--voice-fg')).toBe('#eeeeef');
 expect(applyTheme('future-theme')).toBe('light');
 expect(document.documentElement.style.getPropertyValue('--radius-control')).toBe('17px');
 expect(()=>parseAppearance({themeId:'light',revision:0})).toThrow();
});
it('selection saves through the existing setting owner and returns to the confirmed choice on failure',async()=>{
 const saved:Appearance={themeId:'light',revision:1};
 const save=vi.fn().mockRejectedValue(Error('write denied'));
 const store=new SettingStore(async()=>saved,save);
 await store.load();render(<AppearanceSettings store={store}/>);
 await act(async()=>fireEvent.click(screen.getByRole('button',{name:'石墨银灰'})));
 expect(save).toHaveBeenCalledOnce();
 expect(screen.getByRole('button',{name:'浅色'})).toHaveAttribute('aria-pressed','true');
 expect(screen.getByRole('alert')).toHaveTextContent('保存失败');
});
