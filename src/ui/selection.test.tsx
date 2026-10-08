import {useState} from 'react';
import {act,fireEvent,render,screen} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {afterEach,expect,it,vi} from 'vitest';
import {Switch,SingleChoice} from './selection';
import {advance,body} from './selection-motion';
const options=[{value:'toggle',label:'点按'},{value:'hold',label:'按住'}];
afterEach(()=>vi.restoreAllMocks());
it('controlled switch acts immediately, reverses without jumping, snaps external rollback and releases frames',()=>{
 let now=0,id=0;const callbacks=new Map<number,FrameRequestCallback>();
 vi.spyOn(performance,'now').mockImplementation(()=>now);
 vi.spyOn(window,'requestAnimationFrame').mockImplementation(cb=>{callbacks.set(++id,cb);return id;});
 vi.spyOn(window,'cancelAnimationFrame').mockImplementation(id=>{callbacks.delete(id);});
 const tick=()=>act(()=>{now+=16;const batch=[...callbacks.values()];callbacks.clear();batch.forEach(cb=>cb(now));});
 function Fixture(){const [checked,setChecked]=useState(false);return <><label htmlFor="switch">保存历史</label><Switch id="switch" label="保存历史" checked={checked} onCheckedChange={setChecked}/><button onClick={()=>setChecked(false)}>恢复</button></>;}
 const view=render(<Fixture/>),control=screen.getByRole('switch'),thumb=control.firstElementChild as HTMLElement;
 expect(callbacks.size).toBe(0);
 fireEvent.click(screen.getByText('保存历史'));expect(control).toBeChecked();tick();tick();
 const moving=thumb.style.transform;expect(moving).not.toBe('translateX(20px) scale(1,1)');
 fireEvent.click(control);expect(control).not.toBeChecked();expect(thumb.style.transform).toBe(moving);expect(callbacks.size).toBe(1);
 tick();fireEvent.click(control);fireEvent.click(screen.getByText('恢复'));
 expect(thumb.style.transform).toBe('translateX(0px) scale(1,1)');expect(callbacks.size).toBe(0);
 fireEvent.click(control);view.unmount();expect(callbacks.size).toBe(0);
});
it('rejected operation cannot animate a later external value and radio uses native keys',async()=>{
 const raf=vi.spyOn(window,'requestAnimationFrame');
 const change=vi.fn();const view=render(<Switch checked={false} label="开关" onCheckedChange={change}/>);
 fireEvent.click(screen.getByRole('switch'));expect(change).toHaveBeenCalledWith(true);expect(screen.getByRole('switch')).not.toBeChecked();
 await act(async()=>{});view.rerender(<Switch checked label="开关" onCheckedChange={change}/>);expect(raf).not.toHaveBeenCalled();view.unmount();
 function Fixture(){const [value,setValue]=useState('toggle');return <SingleChoice label="录音方式" value={value} onValueChange={setValue} options={options}/>;}
 render(<Fixture/>);screen.getByLabelText('点按').focus();await userEvent.setup().keyboard('{ArrowRight}');expect(screen.getByLabelText('按住')).toBeChecked();expect(raf).toHaveBeenCalled();
});
it('spring retains velocity on reversal and converges after dropped frames',()=>{
 const state=body([0]);state.target=[20];advance(state,.032,360);const velocity=state.velocity[0]!;
 expect(velocity).toBeGreaterThan(0);state.target=[0];expect(state.velocity[0]).toBe(velocity);
 for(let i=0;i<120;i++)advance(state,.016,360);
 expect(Math.abs(state.position[0]!)).toBeLessThan(.001);expect(Math.abs(state.velocity[0]!)).toBeLessThan(.001);
});
