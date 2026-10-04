import type { ReactNode, MouseEventHandler } from 'react';
import { NavLink } from 'react-router-dom';
import { Icon, type IconName } from './Icon';
import { Button } from './controls';
import s from './ui.module.css';
export function NavItem({ to, icon, children, onClick }: { to: string; icon: IconName; children: ReactNode; onClick?:MouseEventHandler<HTMLAnchorElement> }) {
  return <NavLink to={to} onClick={onClick} end className={({ isActive }) => `${s.nav} ${isActive ? s.navSelected : ''}`}>{({isActive}) => <><Icon name={icon} size={18} signal={isActive} /><span>{children}</span></>}</NavLink>;
}
export function ModeRow({ icon, title, description, buttonLabel, onClick }: { icon: IconName; title: string; description: string; buttonLabel: string; onClick: () => void }) {
  return <div className={s.mode}><Icon name={icon} size={20} /><div className={s.modeCopy}><h3>{title}</h3><p>{description}</p></div><Button className={s.keyButton} onClick={onClick}>{buttonLabel}</Button></div>;
}
