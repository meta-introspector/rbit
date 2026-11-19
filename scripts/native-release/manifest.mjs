/** Signed release checksums. Trust comes from a separately pinned Ed25519 public key. */
import { createHash, createPrivateKey, createPublicKey, sign, verify } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { lstat, readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

const ARCHIVE = /^rbit-[0-9A-Za-z.+-]+-(?:x86_64|aarch64)-(?:unknown-linux-gnu|apple-darwin|pc-windows-msvc)\.(?:tar\.gz|zip)$/u;
const TAG = /^v(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/u;
const MAX_ARCHIVE = 1024 * 1024 * 1024;

export function releaseIdentity(tag, sha) {
  if (typeof tag !== 'string' || tag.length > 128 || !TAG.test(tag) || !/^[a-f0-9]{40}$/u.test(sha ?? '')) {
    throw new Error('Expected a semantic-version v-tag and a full commit SHA');
  }
  return { format: 1, tag, commit: sha };
}

async function regularFile(path, maximum) {
  const stat = await lstat(path);
  if (!stat.isFile() || stat.size > maximum || stat.size === 0) throw new Error('Invalid release file');
  return stat.size;
}

export async function sha256File(path) {
  await regularFile(path, MAX_ARCHIVE);
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest('hex');
}

async function smallFile(path, maximum) {
  await regularFile(path, maximum);
  return readFile(path);
}

function publicKey(pem) {
  const key = createPublicKey(pem);
  if (key.asymmetricKeyType !== 'ed25519') throw new Error('Release key must be Ed25519');
  return key;
}

export async function signDirectory(directory, tag, sha, privatePem, trustedPublicPem) {
  const identity = releaseIdentity(tag, sha);
  const key = createPrivateKey(privatePem);
  if (key.asymmetricKeyType !== 'ed25519') throw new Error('Release key must be Ed25519');
  const derived = createPublicKey(key).export({ type: 'spki', format: 'der' });
  if (!derived.equals(publicKey(trustedPublicPem).export({ type: 'spki', format: 'der' }))) throw new Error('Signing key does not match trusted key');
  const archives = (await readdir(directory)).filter((name) => ARCHIVE.test(name)).sort();
  if (!archives.length || archives.length > 16) throw new Error('Expected one to sixteen platform archives');
  if (archives.some((name) => !name.startsWith(`rbit-${tag.slice(1)}-`))) throw new Error('Archive version differs from tag');
  await writeFile(join(directory, 'RELEASE.json'), `${JSON.stringify(identity)}\n`, { flag: 'wx' });
  const names = [...archives, 'RELEASE.json'].sort();
  const sums = (await Promise.all(names.map(async (name) => `${await sha256File(join(directory, name))}  ${name}\n`))).join('');
  const signature = sign(null, Buffer.from(sums), key);
  if (!verify(null, Buffer.from(sums), publicKey(trustedPublicPem), signature)) throw new Error('Signing key does not match trusted key');
  await writeFile(join(directory, 'SHA256SUMS'), sums, { flag: 'wx' });
  await writeFile(join(directory, 'SHA256SUMS.sig'), signature, { flag: 'wx' });
  return verifyDirectory(directory, trustedPublicPem, identity);
}

export async function verifyDirectory(directory, trustedPublicPem, expected) {
  const sums = await smallFile(join(directory, 'SHA256SUMS'), 16 * 1024);
  const signature = await smallFile(join(directory, 'SHA256SUMS.sig'), 64);
  if (signature.length !== 64 || !verify(null, sums, publicKey(trustedPublicPem), signature)) throw new Error('Invalid release signature');
  const lines = sums.toString('utf8').split('\n');
  if (lines.pop() !== '' || lines.length < 2 || lines.length > 17) throw new Error('Invalid checksum manifest');
  const entries = [];
  for (const line of lines) {
    const match = /^([a-f0-9]{64})  ([A-Za-z0-9.+_-]+)$/u.exec(line);
    if (!match || !(ARCHIVE.test(match[2]) || match[2] === 'RELEASE.json')) throw new Error('Invalid manifest path');
    const [, digest, name] = match;
    if (entries.some((entry) => entry.name === name) || await sha256File(join(directory, name)) !== digest) throw new Error('Release checksum mismatch');
    entries.push({ name, digest, size: await regularFile(join(directory, name), MAX_ARCHIVE) });
  }
  if (!entries.some((entry) => entry.name === 'RELEASE.json')) throw new Error('Missing signed release identity');
  const identity = JSON.parse(await smallFile(join(directory, 'RELEASE.json'), 1024));
  const valid = releaseIdentity(identity.tag, identity.commit);
  if (identity.format !== valid.format || (expected && (identity.tag !== expected.tag || identity.commit !== expected.commit))) throw new Error('Release identity mismatch');
  if (entries.some(({ name }) => name !== 'RELEASE.json' && !name.startsWith(`rbit-${identity.tag.slice(1)}-`))) throw new Error('Archive version mismatch');
  for (const name of ['SHA256SUMS', 'SHA256SUMS.sig']) entries.push({ name, digest: await sha256File(join(directory, name)), size: await regularFile(join(directory, name), 16 * 1024) });
  const allowed = new Set(entries.map(({ name }) => name));
  if ((await readdir(directory)).some((name) => !allowed.has(name))) throw new Error('Unlisted files in release directory');
  return { identity, entries };
}
