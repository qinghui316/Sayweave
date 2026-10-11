import mark from '../assets/brand.svg';
import lettering from '../assets/wordmark.svg';
import s from './brand.module.css';

export function Brand() {
  return <span className={s.brand} role="img" aria-label="Sayweave">
    <img className={s.mark} src={mark} alt="" width={24} height={24}/>
    <img className={s.lettering} src={lettering} alt="" width={112} height={25}/>
  </span>;
}
