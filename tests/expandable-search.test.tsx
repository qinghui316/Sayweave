import {useState} from 'react';
import {describe,it,expect} from 'vitest';
import {render,screen,fireEvent} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import {ExpandableSearch} from '../src/ui/ExpandableSearch';

function Fixture({initial=''}:{initial?:string}){
 const [value,setValue]=useState(initial);
 return <><ExpandableSearch value={value} onValueChange={setValue} label="搜索记录"/><button>其他操作</button></>;
}
describe('shared expandable search',()=>{
 it('focuses on open, preserves active query outside and clears/restores focus with Escape',async()=>{
  const user=userEvent.setup();render(<Fixture/>);
  await user.click(screen.getByRole('button',{name:'搜索记录'}));const field=screen.getByRole('textbox');expect(field).toHaveFocus();
  await user.type(field,'正文');await user.click(screen.getByRole('button',{name:'其他操作'}));expect(field).toHaveValue('正文');
  await user.click(field);await user.keyboard('{Escape}');expect(screen.queryByRole('textbox')).not.toBeInTheDocument();expect(screen.getByRole('button',{name:'搜索记录'})).toHaveFocus();
  await user.click(screen.getByRole('button',{name:'搜索记录'}));await user.click(screen.getByRole('button',{name:'其他操作'}));expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
 });
 it('restores a query without stealing focus and leaves IME Escape to the input method',async()=>{
  const user=userEvent.setup();render(<Fixture initial="已有筛选"/>);const field=screen.getByRole('textbox');expect(field).toHaveValue('已有筛选');expect(field).not.toHaveFocus();
  await user.click(field);fireEvent.compositionStart(field);fireEvent.keyDown(field,{key:'Escape',isComposing:true});expect(field).toHaveValue('已有筛选');
  fireEvent.compositionEnd(field);await user.clear(field);expect(field).toHaveFocus();expect(field).toBeInTheDocument();await user.keyboard('{Escape}');expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
 });
});
