import {useState} from 'react';
import {act,fireEvent,render,screen} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {afterEach,expect,it,vi} from 'vitest';
import {Switch,SingleChoice} from './selection';
const options=[{value:'toggle',label:'点按'},{value:'hold',label:'按住'}];
afterEach(()=>{vi.restoreAllMocks();Object.defineProperty(Element.prototype,'animate',{configurable:true,value:undefined});});
it('controlled switch acts immediately, cancels old motion and never animates external rollback',async()=>{
 const cancel=vi.fn(),animate=vi.fn(()=>({cancel}));Object.defineProperty(Element.prototype,'animate',{configurable:true,value:animate});
 function Fixture(){const [checked,setChecked]=useState(false);return <><label htmlFor="switch">保存历史</label><Switch id="switch" label="保存历史" checked={checked} onCheckedChange={setChecked}/><button onClick={()=>setChecked(false)}>恢复</button></>;}
 const view=render(<Fixture/>);expect(animate).not.toHaveBeenCalled();
 fireEvent.click(screen.getByText('保存历史'));expect(screen.getByRole('switch')).toBeChecked();expect(animate).toHaveBeenCalledTimes(1);
 fireEvent.click(screen.getByRole('switch'));expect(screen.getByRole('switch')).not.toBeChecked();expect(cancel).toHaveBeenCalledTimes(1);
 fireEvent.click(screen.getByRole('switch'));fireEvent.click(screen.getByText('恢复'));expect(animate).toHaveBeenCalledTimes(3);expect(cancel).toHaveBeenCalledTimes(3);
 view.unmount();await act(async()=>{});
});
it('rejected operation cannot animate a later external value and radio uses native keys',async()=>{
 const animate=vi.fn(()=>({cancel:vi.fn()}));Object.defineProperty(Element.prototype,'animate',{configurable:true,value:animate});
 const change=vi.fn();const view=render(<Switch checked={false} label="开关" onCheckedChange={change}/>);
 fireEvent.click(screen.getByRole('switch'));expect(change).toHaveBeenCalledWith(true);expect(screen.getByRole('switch')).not.toBeChecked();
 await act(async()=>{});view.rerender(<Switch checked label="开关" onCheckedChange={change}/>);expect(animate).not.toHaveBeenCalled();view.unmount();
 function Fixture(){const [value,setValue]=useState('toggle');return <SingleChoice label="录音方式" value={value} onValueChange={setValue} options={options}/>;}
 render(<Fixture/>);screen.getByLabelText('点按').focus();await userEvent.setup().keyboard('{ArrowRight}');expect(screen.getByLabelText('按住')).toBeChecked();expect(animate).toHaveBeenCalledTimes(1);
});
