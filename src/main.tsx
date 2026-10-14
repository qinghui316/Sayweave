import {applyTheme} from './design/themes';
import {parseAppearance} from './contracts/appearance';
import React from 'react';
import ReactDOM from 'react-dom/client';
import '@fontsource/noto-sans-sc/400.css';
import '@fontsource/noto-sans-sc/500.css';
import '@fontsource/noto-sans-sc/700.css';
import '@fontsource/inter/600.css';
import './design/tokens.css';
import './design/base.css';
import { App } from './app/App';

document.documentElement.dataset.surface = new URLSearchParams(location.search).get('surface') === 'overlay' ? 'overlay' : 'main';
let initialTheme='light';
try{const native=(window as unknown as {__SAYWEAVE_APPEARANCE__?:unknown}).__SAYWEAVE_APPEARANCE__;initialTheme=parseAppearance(native??JSON.parse(localStorage.getItem('sayweave.appearance')??'null')).themeId;}catch{/* First run uses the existing light palette. */}
applyTheme(initialTheme);
ReactDOM.createRoot(document.getElementById('root')!).render(<React.StrictMode><App /></React.StrictMode>);
