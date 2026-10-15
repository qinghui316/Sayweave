import {applyTheme} from './design/themes';
import {initialAppearanceTheme} from './adapters/appearance-tauri';
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
applyTheme(initialAppearanceTheme());
ReactDOM.createRoot(document.getElementById('root')!).render(<React.StrictMode><App /></React.StrictMode>);
