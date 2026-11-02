// Shared-password login with an HMAC-signed, HttpOnly session cookie.

import crypto from 'node:crypto';

const COOKIE = 'ml_session';
const MAX_AGE_S = 60 * 60 * 24 * 180; // 180 days; this is meant to live on a phone

const sign = (secret, value) =>
  crypto.createHmac('sha256', secret).update(value).digest('base64url');

function safeEqual(a, b) {
  const ab = Buffer.from(String(a));
  const bb = Buffer.from(String(b));
  return ab.length === bb.length && crypto.timingSafeEqual(ab, bb);
}

function readCookie(req, name) {
  for (const part of (req.headers.cookie || '').split(';')) {
    const [k, ...v] = part.trim().split('=');
    if (k === name) return decodeURIComponent(v.join('='));
  }
  return null;
}

export function createAuth({ appPassword, sessionSecret }) {
  const enabled = Boolean(appPassword);

  const isAuthed = (req) => {
    if (!enabled) return true;
    const raw = readCookie(req, COOKIE);
    if (!raw) return false;
    const [exp, sig] = raw.split('.');
    return Number(exp) > Date.now() / 1000 && safeEqual(sig, sign(sessionSecret, exp));
  };

  return {
    enabled,
    isAuthed,
    require(req, res, next) {
      if (isAuthed(req)) return next();
      res.status(401).json({ error: 'login required' });
    },
    login(req, res) {
      if (enabled && !safeEqual(req.body?.password ?? '', appPassword)) {
        return res.status(401).json({ error: 'wrong password' });
      }
      const exp = String(Math.floor(Date.now() / 1000) + MAX_AGE_S);
      const secure = req.secure || req.headers['x-forwarded-proto'] === 'https';
      res.setHeader(
        'Set-Cookie',
        `${COOKIE}=${exp}.${sign(sessionSecret, exp)}; Path=/; HttpOnly; SameSite=Lax; Max-Age=${MAX_AGE_S}${secure ? '; Secure' : ''}`,
      );
      res.json({ ok: true });
    },
    logout(_req, res) {
      res.setHeader('Set-Cookie', `${COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0`);
      res.json({ ok: true });
    },
  };
}
