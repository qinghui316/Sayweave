import {describe,expect,it} from 'vitest';
// @ts-expect-error Native JavaScript acceptance helper.
import {boundaryCheckpoint,preciseProcessRecord} from '../scripts/windows-product-boundaries.mjs';
describe('named installer interruption checkpoints',()=>{
 it('requires both actual prepared records',()=>{expect(boundaryCheckpoint('prepared',{user:{phase:'prepared'},machine:{phase:'prepared'}})).toBe(true);expect(boundaryCheckpoint('prepared',{user:{phase:'committed'},machine:{phase:'prepared'}})).toBe(false);});
 it('does not mislabel a program-copy failure as a machine-commit interruption',()=>{const value={user:{phase:'recovery',lastFailure:{phase:'programInstalled',code:'acceptanceInstallerProgramFailure'}},machine:{phase:'prepared'}};expect(boundaryCheckpoint('program-recovery',value)).toBe(true);expect(boundaryCheckpoint('machine-recovery',value)).toBe(false);value.user.lastFailure.code='otherFailure';expect(boundaryCheckpoint('program-recovery',value)).toBe(false);});
 it('requires persisted user initialization intent after real machine commit',()=>{const value={user:{phase:'recovery',lastFailure:{phase:'machineCommitted',code:'componentUserInitialize'},userInitializationStarted:false},machine:{phase:'machineCommitted'}};expect(boundaryCheckpoint('machine-recovery',value)).toBe(false);value.user.userInitializationStarted=true;expect(boundaryCheckpoint('machine-recovery',value)).toBe(true);});
 it('keeps native FILETIME exact beyond JavaScript integer precision',()=>{const value=preciseProcessRecord('{"server":{"pid":123,"created":134012345678901234,"imageHash":"'+ 'a'.repeat(64)+'"}}','server');expect(value.created).toBe('134012345678901234');expect(()=>preciseProcessRecord('{}','server')).toThrow();});
});
