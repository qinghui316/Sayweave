import { Link } from 'react-router-dom';
import { EmptyState } from '../../ui';
import { zh } from '../../locales/zh-CN';
export function Preferences() { return <EmptyState title={zh.navigation.preferences} description={zh.placeholder.description}><Link to="/">{zh.placeholder.back} →</Link></EmptyState>; }
