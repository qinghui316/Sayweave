import {expect,it} from 'vitest';
import {initialSpeech,isQuietLiveTerminal,type SpeechSnapshot} from '../src/contracts/speech';
const terminal:SpeechSnapshot={...initialSpeech(),taskId:'task',source:'trial',phase:'failed',error:'emptyText'};
it('quietly projects a normal empty live end without changing its diagnostic identity',()=>{
 expect(isQuietLiveTerminal(terminal)).toBe(true);expect(terminal.phase).toBe('failed');
 expect(isQuietLiveTerminal({...terminal,phase:'cancelled',error:'cancelled'})).toBe(true);
 expect(isQuietLiveTerminal({...terminal,phase:'completed',error:null})).toBe(true);
});
it.each([{source:'history'},{retry:true},{error:'network'},{phase:'recording'},{text:'保留文字'},{taskId:null}] as Partial<SpeechSnapshot>[])('does not hide a real error, history retry, active or nonempty result %j',patch=>{
 expect(isQuietLiveTerminal({...terminal,...patch})).toBe(false);
});

it('storage failure does not turn an empty live end into an input warning',()=>{
 const value={...terminal,saveError:true};expect(isQuietLiveTerminal(value)).toBe(true);expect(value.saveError).toBe(true);
});
