import { defineConfig } from '@playwright/test';
export default defineConfig({ testDir: './tests/e2e', use: { baseURL: 'http://127.0.0.1:5173', viewport: {width:1440,height:1024}, screenshot:'only-on-failure' }, workers:1, reporter:[['list'],['html',{open:'never'}]], webServer:{command:'npm run dev',url:'http://127.0.0.1:5173',reuseExistingServer:!process.env.CI,timeout:30000} });
