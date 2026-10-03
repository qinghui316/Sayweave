import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { Button, Field, TextInput, PasswordInput } from './controls';
it('labels errors and keeps disabled/loading actions inert',async()=>{
  const onClick=vi.fn();render(<><Field label="名称" error="无效">{props=><TextInput {...props}/>}</Field><Button loading onClick={onClick}>提交</Button></>);
  expect(screen.getByLabelText('名称')).toHaveAttribute('aria-invalid','true');expect(screen.getByRole('alert')).toHaveTextContent('无效');
  await userEvent.click(screen.getByRole('button'));expect(onClick).not.toHaveBeenCalled();
});
it('reveals password only on explicit action',async()=>{
  render(<PasswordInput aria-label="Key" defaultValue="demo"/>);expect(screen.getByLabelText('Key')).toHaveAttribute('type','password');
  await userEvent.click(screen.getByRole('button'));expect(screen.getByLabelText('Key')).toHaveAttribute('type','text');
});
