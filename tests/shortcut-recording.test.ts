import {describe,it,expect} from 'vitest';
import {ShortcutRecording} from '../src/features/settings/shortcut-recording';
const key=(code:string,type='keydown',mods:Partial<KeyboardEvent>={})=>({code,type,repeat:false,ctrlKey:false,altKey:false,shiftKey:false,metaKey:false,...mods});
describe('focused settings shortcut gesture',()=>{
 it.each([[1,'MouseMiddle'],[3,'MouseX1'],[4,'MouseX2']])('records mouse button %s only on its matching release',(button,name)=>{
  const recording=new ShortcutRecording(),mouse=(type:string,b=button)=>({type,button:b,ctrlKey:false,altKey:false,shiftKey:false,metaKey:false});
  expect(recording.acceptMouse(mouse('mouseup'))).toBeNull();
  expect(recording.acceptMouse(mouse('mousedown'))).toBeNull();
  expect(recording.acceptMouse(mouse('mouseup',0))).toBeNull();
  expect(recording.acceptMouse(mouse('mouseup'))).toBe(name);
 });
 it.each(['ctrlKey','altKey','shiftKey','metaKey'])('preserves %s instead of silently stripping a mouse combination',modifier=>{
  const recording=new ShortcutRecording(),event={type:'mousedown',button:3,ctrlKey:false,altKey:false,shiftKey:false,metaKey:false,[modifier]:true};
  recording.acceptMouse(event);expect(recording.acceptMouse({...event,type:'mouseup'})).toMatch(/\+MouseX1$/);
 });
 it('cannot complete a pending mouse capture from a keyboard release',()=>{
  const recording=new ShortcutRecording();recording.acceptMouse({type:'mousedown',button:4,ctrlKey:false,altKey:false,shiftKey:false,metaKey:false});
  recording.accept(key('ControlLeft'));expect(recording.accept(key('ControlLeft','keyup'))).toBeNull();
  expect(recording.acceptMouse({type:'mouseup',button:4,ctrlKey:false,altKey:false,shiftKey:false,metaKey:false})).toBe('ControlLeft+MouseX2');
 });
 it.each([['ShiftRight','AltRight'],['AltRight','ShiftRight']])('retains Alt plus Shift when %s is released first',first=>{
  const recording=new ShortcutRecording();
  expect(recording.accept(key('AltRight'))).toBeNull();
  expect(recording.accept(key('ShiftRight','keydown',{altKey:true,shiftKey:true}))).toBeNull();
  expect(recording.accept(key(first,'keyup'))).toBe('AltRight+ShiftRight');
 });
 it('retains registered Control+F8 even when Control releases first',()=>{
  const recording=new ShortcutRecording();recording.accept(key('ControlLeft'));
  recording.accept(key('F8','keydown',{ctrlKey:true}));
  expect(recording.accept(key('ControlLeft','keyup'))).toBe('Control+F8');
 });
 it('does not infer a candidate from an unmatched up or repeat',()=>{
  const recording=new ShortcutRecording();
  expect(recording.accept(key('F8','keyup'))).toBeNull();
  recording.accept(key('F8','keydown',{repeat:true}));
  expect(recording.accept(key('F8','keyup'))).toBeNull();
 });
 it('preserves rejected candidates for the existing save validation',()=>{
  for(const [code,mods,expected] of [['Escape',{ctrlKey:true},'Control+Escape'],['KeyA',{altKey:true},'Alt+KeyA'],['F12',{},'F12']] as const){
   const recording=new ShortcutRecording();recording.accept(key(code,'keydown',mods));
   expect(recording.accept(key(code,'keyup'))).toBe(expected);
  }
 });
 it('does not turn a larger Alt Shift chord into the modifier pair',()=>{
  const recording=new ShortcutRecording();recording.accept(key('AltRight'));recording.accept(key('ShiftRight'));
  recording.accept(key('KeyA','keydown',{altKey:true,shiftKey:true}));
  expect(recording.accept(key('KeyA','keyup'))).toBe('Alt+Shift+KeyA');
 });
});
