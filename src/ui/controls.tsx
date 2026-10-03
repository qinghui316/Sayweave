import { useId, useLayoutEffect, useRef, useState, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode, type Ref } from 'react';
import { AnimatedIcon } from './AnimatedIcon';
import { useMotion } from './useMotion';
import * as SelectPrimitive from '@radix-ui/react-select';
import * as SwitchPrimitive from '@radix-ui/react-switch';
import * as TabsPrimitive from '@radix-ui/react-tabs';
import { zh } from '../locales/zh-CN';
import s from './ui.module.css';

export function Button({ variant = 'secondary', loading = false, children, className = '', disabled, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: 'primary'|'secondary'|'quiet'; loading?: boolean }) {
  return <button type="button" {...props} disabled={disabled || loading} aria-busy={loading || undefined} className={`${s.button} ${s[variant]} ${className}`}>{loading && <LoadingIndicator />}{children}</button>;
}
export function IconButton({ label, children, ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { label: string }) {
  return <button type="button" {...props} aria-label={label} title={label} className={`${s.iconButton} ${props.className ?? ''}`}>{children}</button>;
}
export function LoadingIndicator() { const { ref, active } = useMotion(); return <span ref={ref} aria-hidden="true" className={s.loader} style={{ animationPlayState: active ? 'running' : 'paused' }} />; }
export function Field({ label, description, error, children }: { label: string; description?: string; error?: string; children: (props: { id: string; 'aria-describedby'?: string; 'aria-invalid': boolean }) => ReactNode }) {
  const id = useId(); const details = `${id}-detail`;
  return <div className={s.field}><label htmlFor={id}>{label}</label>{children({ id, 'aria-describedby': description || error ? details : undefined, 'aria-invalid': !!error })}{(description || error) && <p id={details} className={s.meta} role={error ? 'alert' : undefined}>{error && <span aria-hidden="true">! </span>}{error ?? description}</p>}</div>;
}
export function TextInput(props: InputHTMLAttributes<HTMLInputElement> & { ref?: Ref<HTMLInputElement> }) { return <input {...props} className={`${s.input} ${props.className ?? ''}`} />; }
export function PasswordInput(props: InputHTMLAttributes<HTMLInputElement>) {
  const [visible, setVisible] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const selection = useRef<[number | null, number | null, 'forward'|'backward'|'none'|null] | null>(null);
  useLayoutEffect(() => { const caret = selection.current; if (caret && input.current) input.current.setSelectionRange(caret[0], caret[1], caret[2] ?? undefined); }, [visible]);
  function toggle() {
    if (input.current) selection.current = [input.current.selectionStart, input.current.selectionEnd, input.current.selectionDirection];
    setVisible(value => !value);
  }
  return <div className={s.password}><TextInput {...props} ref={input} type={visible ? 'text' : 'password'} /><IconButton disabled={props.disabled} label={visible ? zh.dev.hidePassword : zh.dev.showPassword} onClick={toggle} aria-pressed={visible}><AnimatedIcon icon={visible ? 'eyeOff' : 'eye'} size={20}/></IconButton></div>;
}
export function Select({ value, onValueChange, options, label, id }: { value: string; onValueChange: (value: string) => void; options: { value: string; label: string }[]; label: string; id?: string }) {
  return <SelectPrimitive.Root value={value} onValueChange={onValueChange}><SelectPrimitive.Trigger className={s.select} aria-label={label} id={id}><SelectPrimitive.Value /><SelectPrimitive.Icon>⌄</SelectPrimitive.Icon></SelectPrimitive.Trigger><SelectPrimitive.Portal><SelectPrimitive.Content className={s.selectContent} position="popper" sideOffset={6}><SelectPrimitive.Viewport>{options.map(o => <SelectPrimitive.Item key={o.value} value={o.value} className={s.selectItem}><SelectPrimitive.ItemText>{o.label}</SelectPrimitive.ItemText><SelectPrimitive.ItemIndicator>✓</SelectPrimitive.ItemIndicator></SelectPrimitive.Item>)}</SelectPrimitive.Viewport></SelectPrimitive.Content></SelectPrimitive.Portal></SelectPrimitive.Root>;
}
export function Switch({ checked, onCheckedChange, label }: { checked: boolean; onCheckedChange: (value: boolean) => void; label: string }) {
  return <SwitchPrimitive.Root className={s.switch} checked={checked} onCheckedChange={onCheckedChange} aria-label={label}><SwitchPrimitive.Thumb className={s.thumb} /></SwitchPrimitive.Root>;
}
export function Tabs({ value, onValueChange, options, children, label }: { value: string; onValueChange: (value: string) => void; options: {value: string; label: string}[]; children: ReactNode; label: string }) {
  return <TabsPrimitive.Root value={value} onValueChange={onValueChange}><TabsPrimitive.List className={s.tabs} aria-label={label}>{options.map(o => <TabsPrimitive.Trigger key={o.value} value={o.value} className={s.tab}>{o.label}</TabsPrimitive.Trigger>)}</TabsPrimitive.List><TabsPrimitive.Content value={value}>{children}</TabsPrimitive.Content></TabsPrimitive.Root>;
}
export function InlineMessage({ children, status = 'info' }: { children: ReactNode; status?: 'info'|'error'|'success' }) {
  return <p className={s.message} role={status === 'error' ? 'alert' : 'status'}><span aria-hidden="true" className={status === 'success' ? s.confirm : ''}>{status === 'error' ? '!' : status === 'success' ? '✓' : 'i'}</span>{children}</p>;
}
export function EmptyState({ title, description, children }: { title: string; description?: string; children?: ReactNode }) {
  return <section className={s.empty}><h2>{title}</h2>{description && <p>{description}</p>}{children}</section>;
}
export function Keycap({ children }: { children: ReactNode }) { return <kbd className={s.keycap}>{children}</kbd>; }
