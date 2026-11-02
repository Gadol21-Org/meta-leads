import webpush from 'web-push';

export function createPusher(cfg, store, log = console) {
  const enabled = Boolean(cfg.publicKey && cfg.privateKey);
  if (enabled) webpush.setVapidDetails(cfg.subject, cfg.publicKey, cfg.privateKey);

  return {
    enabled,
    publicKey: cfg.publicKey,

    /** Sends one notification to every subscribed device; prunes dead ones. */
    async broadcast(payload) {
      if (!enabled) return { sent: 0, failed: 0 };
      const body = JSON.stringify(payload);
      let sent = 0;
      let failed = 0;
      await Promise.all(
        store.listSubscriptions().map(async (sub) => {
          try {
            await webpush.sendNotification(sub, body, { TTL: 60 * 60 * 24, urgency: 'high' });
            sent++;
          } catch (err) {
            failed++;
            if (err.statusCode === 404 || err.statusCode === 410) {
              await store.removeSubscription(sub.endpoint);
            } else {
              log.warn?.(`push to ${sub.endpoint.slice(0, 40)}… failed: ${err.message}`);
            }
          }
        }),
      );
      return { sent, failed };
    },
  };
}
