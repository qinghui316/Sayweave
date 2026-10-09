import {describe,it,expect} from 'vitest';
import {BrowserLanguage} from './language-tauri';
import {parseLanguageSettings} from '../contracts/language';
describe('independent learning settings',()=>{
 it('preserves the other feature revision and rejects conflicting combined updates atomically',async()=>{
  const port=new BrowserLanguage();
  const first=await port.settings();expect(first.automaticTerms).toEqual({enabled:true,revision:1});
  const off=await port.save({personalization:{enabled:false,revision:1}});
  expect(off.personalization).toEqual({enabled:false,revision:2});expect(off.automaticTerms).toEqual(first.automaticTerms);
  await expect(port.save({automaticTerms:{enabled:false,revision:1},personalization:{enabled:true,revision:1}})).rejects.toMatchObject({code:'conflict'});
  expect(await port.settings()).toEqual(off);
  const both=await port.save({automaticTerms:{enabled:false,revision:1},personalization:{enabled:true,revision:2}});
  expect(both.automaticTerms).toEqual({enabled:false,revision:2});expect(both.personalization).toEqual({enabled:true,revision:3});
  expect(parseLanguageSettings(both)).toEqual(both);
  expect(await port.save({automaticTerms:{enabled:false,revision:2}})).toEqual(both);
 });
 it('validates each returned revision without deriving one feature from another',()=>{
  const view={automaticTerms:{enabled:true,revision:4},personalization:{enabled:false,revision:8}};
  expect(parseLanguageSettings(view)).toEqual(view);
  expect(()=>parseLanguageSettings({...view,automaticTerms:{enabled:true,revision:0}})).toThrow();
 });
});
