import { Link } from 'react-router-dom';
import { EmptyState } from '../../ui';
import { zh } from '../../locales/zh-CN';
export function History() { return <EmptyState title={zh.navigation.history} description={zh.placeholder.description}><Link to="/">{zh.placeholder.back} →</Link></EmptyState>; }
