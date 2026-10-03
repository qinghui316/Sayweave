import {describe,expect,it} from 'vitest';
import fixture from './dictionary.json';
import {dictionaryKey,parseDictionaryRecord,parseDictionaryStatus,parseDictionaryPage} from '../../src/contracts/dictionary';
describe('dictionary serialization',()=>{
  it('matches shared Rust fixture and normalization keys',()=>{
    expect(parseDictionaryRecord(fixture.record)).toEqual(fixture.record);expect(parseDictionaryStatus(fixture.status)).toEqual(fixture.status);
    fixture.keys.forEach(sample=>expect(dictionaryKey(sample.input)).toBe(sample.key));
  });
  it('rejects malformed native records/pages',()=>{
    expect(()=>parseDictionaryRecord({...fixture.record,revision:0})).toThrow();expect(()=>parseDictionaryPage({records:[],page:0,total:0})).toThrow();
  });
});
