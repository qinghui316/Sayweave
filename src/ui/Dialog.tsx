import { useRef, type ReactNode } from 'react';
import * as Primitive from '@radix-ui/react-dialog';
import { Icon } from './Icon';
import { zh } from '../locales/zh-CN';
import s from './ui.module.css';
export function Dialog({ open, onOpenChange, title, description, children, className = '' }: { open: boolean; onOpenChange: (open: boolean) => void; title: string; description: string; children: ReactNode; className?: string }) {
  const opener = useRef<HTMLElement | null>(null);
  return <Primitive.Root open={open} onOpenChange={onOpenChange}><Primitive.Portal><Primitive.Overlay className={s.overlay} /><Primitive.Content className={`${s.dialog} ${className}`} onOpenAutoFocus={() => { opener.current = document.activeElement as HTMLElement; }} onCloseAutoFocus={event => { event.preventDefault(); opener.current?.focus(); }}><Primitive.Title className={s.srOnly}>{title}</Primitive.Title><Primitive.Description className={s.srOnly}>{description}</Primitive.Description>{children}<Primitive.Close aria-label={zh.settings.close} title={zh.settings.close} className={s.dialogClose}><Icon name="x" size={22}/></Primitive.Close></Primitive.Content></Primitive.Portal></Primitive.Root>;
}
