/**
 * Self-signed certificate for local WebTransport (.dev/cert.pem, .dev/key.pem).
 * Browsers accept it through serverCertificateHashes: ECDSA P-256, valid at most 14 days,
 * so it is regenerated when older than 10 days. Needs `openssl` (Git for Windows has one).
 */
import { existsSync, mkdirSync, statSync } from 'node:fs';
import { join } from 'node:path';

export function ensureDevCert(root = process.cwd()): boolean {
  const dir = join(root, '.dev');
  const cert = join(dir, 'cert.pem');
  const key = join(dir, 'key.pem');
  const fresh = existsSync(cert) && existsSync(key) && Date.now() - statSync(cert).mtimeMs < 10 * 24 * 3600 * 1000;
  if (fresh) return true;
  mkdirSync(dir, { recursive: true });
  const r = Bun.spawnSync(
    [
      'openssl',
      'req',
      '-x509',
      '-newkey',
      'ec',
      '-pkeyopt',
      'ec_paramgen_curve:prime256v1',
      '-days',
      '13',
      '-nodes',
      '-subj',
      '/CN=localhost',
      '-addext',
      'subjectAltName=DNS:localhost,IP:127.0.0.1',
      '-keyout',
      key,
      '-out',
      cert,
    ],
    { env: { ...process.env, MSYS2_ARG_CONV_EXCL: '*' }, stderr: 'pipe' },
  );
  if (r.exitCode !== 0) {
    console.warn('[dev-cert] openssl failed — WebTransport stays off in development:', r.stderr.toString().trim());
    return false;
  }
  console.log('[dev-cert] new certificate in .dev/');
  return true;
}

if (import.meta.main) ensureDevCert();
