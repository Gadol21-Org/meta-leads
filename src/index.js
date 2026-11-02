import { loadConfig } from './config.js';
import { openStore } from './store.js';
import { createApp } from './server.js';

const config = loadConfig();
const store = await openStore(config.dataFile);
const { app } = createApp({ config, store });

const warn = (m) => console.warn(`[config] ${m}`);
if (!config.appPassword) warn('APP_PASSWORD is empty: anyone who can reach the server can read leads.');
if (!config.meta.appSecret) warn('META_APP_SECRET is empty: webhook POSTs will be rejected.');
if (!config.meta.pageAccessToken) warn('META_PAGE_ACCESS_TOKEN is empty: leads will only have webhook ids.');
if (!config.vapid.publicKey) warn('VAPID keys missing: run `npm run vapid` and set them to enable push.');

app.listen(config.port, () => console.log(`meta-leads listening on :${config.port}`));
