// Tiny JSON-file store. Writes are serialised and atomic (write + rename),
// which is plenty for a single-instance deployment with a few thousand leads.
// Swap this module for a real database if you run more than one instance.

import fs from 'node:fs/promises';
import path from 'node:path';

const EMPTY = () => ({ leads: {}, subscriptions: {} });

export async function openStore(file) {
  let data = EMPTY();
  if (file) {
    try {
      data = { ...EMPTY(), ...JSON.parse(await fs.readFile(file, 'utf8')) };
    } catch (err) {
      if (err.code !== 'ENOENT') throw err;
    }
  }

  let chain = Promise.resolve();
  const persist = () => {
    if (!file) return Promise.resolve();
    chain = chain.then(async () => {
      await fs.mkdir(path.dirname(file), { recursive: true });
      const tmp = `${file}.${process.pid}.tmp`;
      await fs.writeFile(tmp, JSON.stringify(data, null, 2));
      await fs.rename(tmp, file);
    });
    return chain;
  };

  return {
    listLeads() {
      return Object.values(data.leads).sort((a, b) =>
        String(b.created_time).localeCompare(String(a.created_time)),
      );
    },
    getLead(id) {
      return data.leads[id] || null;
    },
    hasLead(id) {
      return Boolean(data.leads[id]);
    },
    async saveLead(lead) {
      data.leads[lead.id] = lead;
      await persist();
      return lead;
    },

    listSubscriptions() {
      return Object.values(data.subscriptions);
    },
    async addSubscription(sub) {
      data.subscriptions[sub.endpoint] = { ...sub, addedAt: new Date().toISOString() };
      await persist();
    },
    async removeSubscription(endpoint) {
      delete data.subscriptions[endpoint];
      await persist();
    },
  };
}
