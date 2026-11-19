/** Publish verified release assets through the native REST API, keeping partial uploads private. */
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { join } from 'node:path';
import { verifyDirectory, releaseIdentity } from './manifest.mjs';

export function repositoryEndpoint(origin, repository) {
  const parsed = new URL(origin);
  if (parsed.protocol !== 'https:' || parsed.username || parsed.password || parsed.search || parsed.hash || parsed.pathname !== '/') throw new Error('Release origin must be an HTTPS origin');
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/u.test(repository) || repository.split('/').some((part) => part === '.' || part === '..')) throw new Error('Invalid release repository');
  return `${parsed.origin}/api/v1/repos/${repository}`;
}

async function boundedJson(response) {
  const chunks = [];
  let size = 0;
  for await (const chunk of response.body ?? []) {
    size += chunk.length;
    if (size > 1024 * 1024) throw new Error('Release API response exceeds limit');
    chunks.push(chunk);
  }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'));
}

function apiClient(base, token, fetcher) {
  if (!token || /[\r\n]/u.test(token)) throw new Error('A release token is required');
  return async (path, { method = 'GET', json, body, length, bytes = false, missing = false } = {}) => {
    const headers = { Authorization: `Bearer ${token}`, Accept: bytes ? 'application/octet-stream' : 'application/json' };
    if (json !== undefined) { headers['Content-Type'] = 'application/json'; body = JSON.stringify(json); }
    if (length !== undefined) { headers['Content-Length'] = String(length); headers['Content-Type'] = 'application/octet-stream'; }
    const response = await fetcher(`${base}${path}`, { method, headers, body, redirect: 'error', signal: AbortSignal.timeout(120_000), ...(body && typeof body !== 'string' ? { duplex: 'half' } : {}) });
    if (missing && response.status === 404) { await response.body?.cancel(); return null; }
    if (!response.ok) { await response.body?.cancel(); throw new Error(`Release API ${method} failed with status ${response.status}`); }
    return bytes ? response : boundedJson(response);
  };
}

async function verifyRemoteAsset(api, asset, entry) {
  if (!Number.isSafeInteger(asset.id) || asset.id <= 0 || asset.size !== entry.size) throw new Error('Existing release asset differs');
  const response = await api(`/releases/assets/${asset.id}`, { bytes: true });
  const hash = createHash('sha256');
  let size = 0;
  for await (const chunk of response.body ?? []) {
    size += chunk.length;
    if (size > entry.size) throw new Error('Existing release asset exceeds expected size');
    hash.update(chunk);
  }
  if (size !== entry.size || hash.digest('hex') !== entry.digest) throw new Error('Existing release asset checksum differs');
}

export async function publishRelease(options, fetcher = fetch) {
  const { origin, repository, tag, sha, directory, publicKey, token, notes } = options;
  const expected = releaseIdentity(tag, sha);
  if (typeof notes !== 'string' || !notes.trim() || Buffer.byteLength(notes) > 64 * 1024) throw new Error('Release notes are required and bounded');
  const { entries } = await verifyDirectory(directory, publicKey, expected);
  const api = apiClient(repositoryEndpoint(origin, repository), token, fetcher);
  const checkTag = async () => {
    const commit = await api(`/commits/${encodeURIComponent(`refs/tags/${tag}`)}`);
    if (commit.sha !== sha) throw new Error('Remote tag no longer identifies the tested commit');
  };
  await checkTag();
  let release = await api(`/releases/tags/${encodeURIComponent(tag)}`, { missing: true });
  const prerelease = tag.slice(1).split('+')[0].includes('-');
  if (release && (release.tag_name !== tag || release.body !== notes || release.prerelease !== prerelease)) throw new Error('Existing release metadata differs');
  if (!release) release = await api('/releases', { method: 'POST', json: { tag_name: tag, target_commitish: sha, name: `rbit ${tag.slice(1)}`, body: notes, draft: true, prerelease } });
  if (!Number.isSafeInteger(release.id) || release.id <= 0) throw new Error('Invalid release identifier');
  const assets = await api(`/releases/${release.id}/assets?per_page=100`);
  if (!Array.isArray(assets) || assets.length > entries.length || assets.some((asset) => !entries.some((entry) => entry.name === asset.name))) throw new Error('Unexpected release assets');
  for (const entry of entries) {
    const matching = assets.filter((asset) => asset.name === entry.name);
    if (matching.length > 1) throw new Error('Duplicate release assets');
    if (matching.length) await verifyRemoteAsset(api, matching[0], entry);
    else {
      if (!release.draft) throw new Error('Published release is incomplete; refusing to change it');
      const asset = await api(`/releases/${release.id}/assets?name=${encodeURIComponent(entry.name)}`, { method: 'POST', body: createReadStream(join(directory, entry.name)), length: entry.size });
      await verifyRemoteAsset(api, asset, entry);
    }
  }
  await checkTag();
  if (release.draft) release = await api(`/releases/${release.id}`, { method: 'PATCH', json: { draft: false } });
  if (release.draft || release.tag_name !== tag) throw new Error('Release publication was not confirmed');
  return { id: release.id, tag, commit: sha };
}
