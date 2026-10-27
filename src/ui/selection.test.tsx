import {useState} from 'react';
import {act,fireEvent,render,screen} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {afterEach,expect,it,vi} from 'vitest';
import {Switch,SingleChoice,SelectionHighlight,useSelectionHighlight} from './selection';
import {advance,body} from './selection-motion';
import {RubberIndicator,useRubberSelection} from './RubberSelection';
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
it('model rows share the incumbent spring, remeasure replaced rows and cancel on unmount',async()=>{
 let now=0,id=0;const callbacks=new Map<number,FrameRequestCallback>();
 vi.spyOn(performance,'now').mockImplementation(()=>now);
 vi.spyOn(window,'requestAnimationFrame').mockImplementation(cb=>{callbacks.set(++id,cb);return id;});
 vi.spyOn(window,'cancelAnimationFrame').mockImplementation(key=>{callbacks.delete(key);});
 vi.spyOn(HTMLElement.prototype,'offsetTop','get').mockImplementation(function(this:HTMLElement){return Number(this.dataset.y??0);});
 vi.spyOn(HTMLElement.prototype,'offsetWidth','get').mockReturnValue(160);
 vi.spyOn(HTMLElement.prototype,'offsetHeight','get').mockReturnValue(80);
 vi.spyOn(HTMLElement.prototype,'offsetParent','get').mockImplementation(function(this:HTMLElement){return this.parentElement?.closest('[data-rubber]')??null;});
 const tick=()=>act(()=>{now+=16;const batch=[...callbacks.values()];callbacks.clear();batch.forEach(cb=>cb(now));});
 function Fixture(){const [value,setValue]=useState('cloud'),[rebuilt,setRebuilt]=useState(false),[busy,setBusy]=useState(true);const {ref}=useRubberSelection(value,false,{selector:'[data-model-card][data-selected="true"]',clipRows:'[data-model-card]'});return <><div ref={ref} data-rubber=""><RubberIndicator/><button key={'cloud'+rebuilt} data-model-card="" data-y={rebuilt?92:0} data-selected={value==='cloud'} onClick={()=>setValue('cloud')}>云端</button><section key={'local'+rebuilt}><button data-model-card="" data-y={rebuilt?0:92} data-selected={value==='local'} onClick={()=>setValue('local')}>本地{busy&&<i/>}</button></section></div><button onClick={()=>setRebuilt(current=>!current)}>重建</button><button onClick={()=>setBusy(false)}>结束等待</button></>;}
 const view=render(<Fixture/>),cloud=screen.getByRole('button',{name:'云端'}),local=screen.getByRole('button',{name:'本地'}),layer=view.container.querySelector<HTMLElement>('[data-rubber-indicator]')!;
 expect(layer.style.transform).toBe('translate(0px,0px)');expect(cloud.style.getPropertyValue('--selection-clip')).toBe('inset(0px 0px 0px 0px)');
 fireEvent.click(local);expect(layer.style.transform).toBe('translate(0px,0px)');tick();tick();
 const moving=layer.style.transform;expect(moving).not.toBe('translate(0px,92px)');expect(local.style.getPropertyValue('--selection-clip')).not.toBe('inset(0 0 100%)');
 fireEvent.click(screen.getByRole('button',{name:'结束等待'}));await act(async()=>{});expect(layer.style.transform).toBe(moving);expect(callbacks.size).toBe(1);
 fireEvent.click(cloud);expect(layer.style.transform).toBe(moving);expect(callbacks.size).toBe(1);
 for(let i=0;i<80;i++)tick();expect(layer.style.transform).toBe('translate(0px,0px)');expect(local.style.getPropertyValue('--selection-clip')).toBe('inset(0 0 100%)');
 fireEvent.click(local);tick();fireEvent.click(screen.getByRole('button',{name:'重建'}));await act(async()=>{});
 expect(local.isConnected).toBe(false);expect(callbacks.size).toBe(0);expect(layer.style.transform).toBe('translate(0px,0px)');
 expect(screen.getByRole('button',{name:'本地'}).style.getPropertyValue('--selection-clip')).toBe('inset(0px 0px 0px 0px)');
 fireEvent.click(screen.getByRole('button',{name:'云端'}));view.unmount();expect(callbacks.size).toBe(0);
});
it('shared highlight follows enabled menu rows, ignores disabled rows and releases frames',async()=>{
 let id=0;const callbacks=new Map<number,FrameRequestCallback>();
 vi.spyOn(window,'requestAnimationFrame').mockImplementation(cb=>{callbacks.set(++id,cb);return id;});
 vi.spyOn(window,'cancelAnimationFrame').mockImplementation(key=>{callbacks.delete(key);});
 const tick=()=>act(()=>{const batch=[...callbacks.values()];callbacks.clear();batch.forEach(cb=>cb(16));});
 function Fixture(){const {ref}=useSelectionHighlight('[role="menuitem"]:not([data-disabled])');return <div ref={ref} role="menu"><SelectionHighlight/><button role="menuitem">编辑</button><button role="menuitem">删除</button><button role="menuitem" data-disabled="" disabled>不可用</button></div>;}
 const view=render(<Fixture/>),[first,second,disabled]=screen.getAllByRole('menuitem'),layer=view.container.querySelector<HTMLElement>('[data-selection-highlight]')!;
 for(const [i,row] of [first!,second!,disabled!].entries()){Object.defineProperties(row,{offsetTop:{value:6+i*36},offsetLeft:{value:6},offsetWidth:{value:132},offsetHeight:{value:36}});}
 fireEvent.pointerMove(first!);tick();expect(layer.style.opacity).toBe('1');expect(layer.style.transform).toBe('translate(6px, 6px)');
 fireEvent.pointerMove(disabled!);tick();expect(layer.style.transform).toBe('translate(6px, 6px)');
 act(()=>second!.focus());tick();expect(layer.style.transform).toBe('translate(6px, 42px)');
 await act(async()=>{second!.setAttribute('data-disabled','');});tick();expect(layer.style.opacity).toBe('0');
 fireEvent.pointerMove(first!);expect(callbacks.size).toBe(1);view.unmount();expect(callbacks.size).toBe(0);
});
