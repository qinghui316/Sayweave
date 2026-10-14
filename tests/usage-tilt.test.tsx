import {render,fireEvent} from '@testing-library/react';
import {afterEach,describe,it,expect,vi} from 'vitest';
import {useUsageTilt} from '../src/features/home/use-usage-tilt';

function Fixture({enabled=true}:{enabled?:boolean}){const {area,card}=useUsageTilt(enabled);return <section ref={area} data-testid="area"><div ref={card} data-testid="card"/></section>;}
afterEach(()=>vi.unstubAllGlobals());
describe('usage motion lifecycle',()=>{
 function setup(){
  const queries:EventTarget[]=[];
  vi.stubGlobal('matchMedia',(query:string)=>{const target=Object.assign(new EventTarget(),{matches:query.includes('hover')});queries.push(target);return target;});
  let callback:FrameRequestCallback|undefined;
  const cancel=vi.fn(()=>{callback=undefined;});vi.stubGlobal('requestAnimationFrame',(fn:FrameRequestCallback)=>{callback=fn;return 1;});vi.stubGlobal('cancelAnimationFrame',cancel);
  const view=render(<Fixture/>),area=view.getByTestId('area'),card=view.getByTestId('card');
  vi.spyOn(area,'getBoundingClientRect').mockReturnValue({x:0,y:0,left:0,top:0,right:200,bottom:200,width:200,height:200,toJSON(){return {};}});
  const move=()=>{const event=new MouseEvent('pointermove',{clientX:200,clientY:0,bubbles:true});Object.defineProperty(event,'pointerType',{value:'mouse'});area.dispatchEvent(event);};
  return {view,area,card,move,flush:()=>callback?.(1),queries,cancel};
 }
 it('cancels a queued frame on hiding, pointer cancellation, and unmount',()=>{
  const f=setup();f.move();fireEvent(document,new Event('visibilitychange'));f.flush();expect(f.card).not.toHaveAttribute('data-tilting');
  f.move();f.flush();expect(f.card).toHaveAttribute('data-tilting','true');fireEvent(f.area,new Event('pointercancel'));expect(f.card).not.toHaveAttribute('data-tilting');
  f.move();f.view.unmount();f.flush();expect(f.card).not.toHaveAttribute('data-tilting');expect(f.cancel).toHaveBeenCalled();
 });
 it('resets an active tilt when motion preferences or application motion change',()=>{
  const f=setup();f.move();f.flush();expect(f.card).toHaveAttribute('data-tilting','true');f.queries[0]!.dispatchEvent(new Event('change'));expect(f.card).not.toHaveAttribute('data-tilting');
  f.move();f.view.rerender(<Fixture enabled={false}/>);f.flush();expect(f.card).not.toHaveAttribute('data-tilting');
 });
});
