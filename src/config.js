// All configuration comes from environment variables (see .env.example).
// Node >= 20.6 can load a .env file with `node --env-file=.env src/index.js`.

export function loadConfig(env = process.env) {
  return {
    port: Number(env.PORT || 3000),
    dataFile: env.DATA_FILE || 'data/db.json',
    publicUrl: (env.PUBLIC_URL || '').replace(/\/$/, ''),

    // Access to the PWA itself. Leads are personal data, so the UI and API
    // are behind a shared password. Leave empty only for local development.
    appPassword: env.APP_PASSWORD || '',
    sessionSecret: env.SESSION_SECRET || env.META_APP_SECRET || 'dev-insecure-secret',

    // Meta app + page
    meta: {
      graphVersion: env.META_GRAPH_VERSION || 'v21.0',
      appSecret: env.META_APP_SECRET || '',
      verifyToken: env.META_VERIFY_TOKEN || '',
      pageId: env.META_PAGE_ID || '',
      pageAccessToken: env.META_PAGE_ACCESS_TOKEN || '',
    },

    vapid: {
      publicKey: env.VAPID_PUBLIC_KEY || '',
      privateKey: env.VAPID_PRIVATE_KEY || '',
      subject: env.VAPID_SUBJECT || 'mailto:admin@example.com',
    },

  };
}
