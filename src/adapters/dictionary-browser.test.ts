import {expect,it} from 'vitest';
import {BrowserDictionary} from './dictionary-browser';
import type {DictionaryInput} from '../contracts/dictionary';
const term=(original:string):DictionaryInput=>({kind:'term',original,category:null,replacement:null});
it('deduplicates normalized terms and rejects stale writes',async()=>{
  const port=new BrowserDictionary();const first=await port.create(term(' Figma '));
  await expect(port.create(term('figma'))).rejects.toMatchObject({code:'duplicate',existingId:first.id});
  for(const original of ['a\u2028b','a\u2029b']) await expect(port.create(term(original))).rejects.toMatchObject({code:'invalid'});
  const next=await port.update(term('Sayweave'),first.id,first.revision);expect(next.revision).toBe(2);expect(next.id).toBe(first.id);
  await expect(port.update(term('Stale'),first.id,1)).rejects.toMatchObject({code:'conflict'});await expect(port.delete('term',first.id,1)).rejects.toMatchObject({code:'conflict'});
  await port.delete('term',first.id,2);expect((await port.list({kind:'term',search:'',page:1})).total).toBe(0);
});
it('isolates kinds, treats wildcard literally and clamps pagination',async()=>{
  const port=new BrowserDictionary();for(let i=0;i<55;i++)await port.create(term(`词条${i}`));
  await port.create(term('100%_正确'));expect((await port.list({kind:'term',search:'%_',page:1})).total).toBe(1);
  const page=await port.list({kind:'term',search:'',page:100});expect(page.page).toBe(2);expect(page.records).toHaveLength(6);
  await port.create({kind:'correction',original:'词条0',replacement:'目标',category:null});expect((await port.list({kind:'correction',search:'目标',page:1})).total).toBe(1);
});
