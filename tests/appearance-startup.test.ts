import {afterEach,expect,it,vi} from 'vitest';
import bootstrapSource from '../src/design/appearance-bootstrap.mjs?raw';
import {applyTheme,themeById,startupTheme} from '../src/design/themes';
import type {Appearance} from '../src/contracts/appearance';

const bootstrap=(value:Appearance)=>{
  const run=new Function('window','document',`return ${bootstrapSource}`)(window,document);
  run(value,themeById(value.themeId));
};
afterEach(()=>{delete document.documentElement.dataset.theme;});

it('native startup snapshot cannot overwrite a newer light selection on DOM ready',()=>{
  bootstrap({themeId:'graphite',revision:16});
  expect(document.documentElement.dataset.theme).toBe('graphite');
  applyTheme('light');
  document.dispatchEvent(new Event('DOMContentLoaded'));
  expect(document.documentElement.dataset.theme).toBe('light');
});
it('late native initialization cannot recolor an already initialized page',()=>{
  applyTheme('light');
  bootstrap({themeId:'graphite',revision:16});
  expect(document.documentElement.dataset.theme).toBe('light');
});
it('desktop never consumes browser preview preferences, even without a native snapshot',()=>{
  const preview=vi.fn(()=>JSON.stringify({themeId:'graphite',revision:99}));
  expect(startupTheme(true,undefined,preview)).toBe('light');
  expect(startupTheme(true,{themeId:'light',revision:17},preview)).toBe('light');
  expect(startupTheme(true,{themeId:'graphite',revision:18},preview)).toBe('graphite');
  expect(preview).not.toHaveBeenCalled();
  expect(startupTheme(false,undefined,preview)).toBe('graphite');
});
it('browser and unknown native choices fall back to light',()=>{
  expect(startupTheme(false,undefined,()=>null)).toBe('light');
  expect(startupTheme(true,{themeId:'unknown',revision:2},()=>null)).toBe('light');
  expect(startupTheme(false,undefined,()=>'{')).toBe('light');
});
