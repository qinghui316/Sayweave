import {act, render, screen} from '@testing-library/react';
import {expect, it, vi} from 'vitest';
import {VoiceSurface} from '../src/features/voice';
import type {SpeechMeterFrame, SpeechMeterSource} from '../src/contracts/speech';

it('reconnects real meter frames after editing and detaches on stop',()=>{
 const frames=new Map<number,FrameRequestCallback>();let frameId=0;
 vi.stubGlobal('requestAnimationFrame',(callback:FrameRequestCallback)=>{frames.set(++frameId,callback);return frameId;});
 vi.stubGlobal('cancelAnimationFrame',(id:number)=>{frames.delete(id);});
 vi.stubGlobal('matchMedia',()=>({matches:true,addEventListener:()=>{},removeEventListener:()=>{}}));
 let listener:(()=>void)|undefined;
 let meter:SpeechMeterFrame={taskId:'owned',sequence:1,captureSequence:1,receivedAt:performance.now(),level:.8,energies:Array(15).fill(.8),bars:Array(15).fill(.8),durationMs:50};
 const source:SpeechMeterSource={getSnapshot:()=>meter,subscribe:notify=>{listener=notify;return()=>{listener=undefined;};}};
 const props={phase:'recording' as const,start:()=>{},finish:()=>{},cancel:()=>{},level:.8,meterSource:source};
 const view=render(<VoiceSurface {...props} presentation="editing"/>);
 try {
  expect(listener).toBeUndefined();
  expect(screen.getByRole('group')).toHaveAccessibleName('编辑中，口述已暂停');
  const bar=()=>view.container.querySelector<HTMLElement>('[data-live] i')!;
  const paused=bar().style.transform;
  view.rerender(<VoiceSurface {...props} presentation="recording"/>);
  expect(listener).toBeTypeOf('function');
  expect(bar().style.transform).not.toBe(paused);
  const loud=bar().style.transform;
  act(()=>{
   meter={...meter,sequence:2,captureSequence:2,receivedAt:performance.now(),level:.2,bars:Array(15).fill(.2)};
   listener?.();
   const pending=[...frames.values()];frames.clear();pending.forEach(callback=>callback(performance.now()));
  });
  expect(bar().style.transform).not.toBe(loud);
  view.rerender(<VoiceSurface {...props} presentation="idle"/>);
  expect(listener).toBeUndefined();
  expect(view.container.querySelector('[data-live]')).toBeNull();
 } finally {view.unmount();vi.unstubAllGlobals();}
});
