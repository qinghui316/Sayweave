import { expect, it } from 'vitest';
import { parseSnapshot } from '../../src/contracts/runtime';
import fixture from './snapshot.json';
it('accepts the shared Rust/TypeScript fixture and rejects malformed payloads',()=>{
  expect(parseSnapshot(fixture)).toEqual(fixture);
  expect(()=>parseSnapshot({...fixture,protocol:2})).toThrow();
  expect(()=>parseSnapshot({...fixture,session:{id:-1,phase:'unknown'}})).toThrow();
});
