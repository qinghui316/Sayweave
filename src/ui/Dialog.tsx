import { useRef, type ReactNode } from 'react';
import * as Primitive from '@radix-ui/react-dialog';
import { Icon } from './Icon';
import { zh } from '../locales/zh-CN';
import s from './ui.module.css';
export function Dialog({ open, onOpenChange, title, description, children, className = '', confirmation = false }: { open: boolean; onOpenChange: (open: boolean) => void; title: string; description: string; children: ReactNode; className?: string; confirmation?: boolean }) {
  const opener = useRef<HTMLElement | null>(null);
  return <Primitive.Root open={open} onOpenChange={onOpenChange}><Primitive.Portal><Primitive.Overlay className={`${s.overlay} ${confirmation?s.confirmationOverlay:''}`} /><Primitive.Content role={confirmation?'alertdialog':'dialog'} className={`${s.dialog} ${confirmation?s.confirmationDialog:''} ${className}`} onOpenAutoFocus={() => { opener.current = document.activeElement as HTMLElement; }} onCloseAutoFocus={event => { event.preventDefault(); if(opener.current?.isConnected)opener.current.focus(); }}><Primitive.Title className={confirmation?s.confirmationTitle:s.srOnly}>{title}</Primitive.Title><Primitive.Description className={confirmation?s.confirmationDescription:s.srOnly}>{description}</Primitive.Description>{children}{!confirmation&&<Primitive.Close aria-label={zh.settings.close} title={zh.settings.close} className={s.dialogClose}><Icon name="x" size={22}/></Primitive.Close>}</Primitive.Content></Primitive.Portal></Primitive.Root>;
}
