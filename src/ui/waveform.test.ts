import {expect,it} from 'vitest';
import {WaveEnvelope} from './waveform';
import type {SpeechMeterFrame} from '../contracts/speech';
const frame=(energies:number[],receivedAt=0):SpeechMeterFrame=>({taskId:'one',sequence:1,captureSequence:1,receivedAt,energies,level:0,durationMs:50});
it('keeps independent temporal energy, bounded gain and a fresh silence floor',()=>{
 const wave=new WaveEnvelope(),quiet=frame(Array(15).fill(0));
 const breathing=wave.draw(quiet,100,50,false);
 expect(new Set(breathing).size).toBeGreaterThan(1);expect(Math.max(...breathing)).toBeLessThan(.126);
 const input={...frame(Array.from({length:15},(_,i)=>i%2?.06:0),100),captureSequence:2};
 const speech=wave.draw(input,150,50,false);expect(speech[1]).toBeGreaterThan(speech[0]!);
 expect(Math.max(...speech)).toBeLessThan(1);
 const stale=wave.draw(input,1200,1000,false);
 const stopped=wave.draw(input,2300,1000,false);expect(Math.max(...stopped)).toBeLessThan(Math.max(...stale));
 expect(Math.max(...stopped)).toBeCloseTo(.084,3);
});
it('does not treat duplicate frames as new audio and resets task-local gain',()=>{
 const wave=new WaveEnvelope(),input=frame(Array(15).fill(.05));
 const initial=wave.draw(input,100,50,true);
 expect(wave.draw({...input,energies:Array(15).fill(1)},200,50,true)).toEqual(initial);
 expect(wave.draw(input,1001,50,true)).toEqual(Array(15).fill(.084));
 expect(wave.draw({...input,taskId:'two',energies:Array(15).fill(0),receivedAt:1100},1100,50,true)).toEqual(Array(15).fill(.084));
});
