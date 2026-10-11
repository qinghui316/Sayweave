import { defineConfig } from '@playwright/test';
const port=Number(process.env.SAYWEAVE_E2E_PORT??5173);
if(!Number.isInteger(port)||port<1024||port>65535)throw Error('Invalid Sayweave E2E port');
const url=`http://127.0.0.1:${port}`;
export default defineConfig({ testDir: './tests/e2e', use: { baseURL: url, viewport: {width:1440,height:1024}, screenshot:'only-on-failure' }, workers:1, reporter:[['list'],['html',{open:'never'}]], webServer:{command:`npm run dev -- --port ${port} --strictPort`,url,reuseExistingServer:false,timeout:30000} });
