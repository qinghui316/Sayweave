import {describe,it,expect} from 'vitest';
import {BrowserProcessing} from './processing-tauri';
import {BrowserModels} from './models-browser';
import {parseProcessing} from '../contracts/processing';
describe('independent learning settings',()=>{
 it('preserves the other feature revision and rejects conflicting combined updates atomically',async()=>{
  const port=new BrowserProcessing(new BrowserModels());
  const first=await port.status();expect(first.automaticTerms).toEqual({enabled:true,revision:1});
  const off=await port.saveLanguage({personalization:{enabled:false,revision:1}});
  expect(off.personalization).toEqual({enabled:false,revision:2});expect(off.automaticTerms).toEqual(first.automaticTerms);
  await expect(port.saveLanguage({automaticTerms:{enabled:false,revision:1},personalization:{enabled:true,revision:1}})).rejects.toMatchObject({code:'conflict'});
  expect(await port.status()).toEqual(off);
  const both=await port.saveLanguage({automaticTerms:{enabled:false,revision:1},personalization:{enabled:true,revision:2}});
  expect(both.automaticTerms).toEqual({enabled:false,revision:2});expect(both.personalization).toEqual({enabled:true,revision:3});
  expect(parseProcessing(both)).toEqual(both);
  expect(await port.saveLanguage({automaticTerms:{enabled:false,revision:2}})).toEqual(both);
 });
 it('validates each returned revision without deriving one feature from another',()=>{
  const view={settings:{enabled:true,revision:1},automaticTerms:{enabled:true,revision:4},personalization:{enabled:false,revision:8},available:false,model:null,error:null};
  expect(parseProcessing(view)).toEqual(view);
  expect(()=>parseProcessing({...view,automaticTerms:{enabled:true,revision:0}})).toThrow();
 });
});
