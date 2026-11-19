/** Verify release integrity, resumable private uploads and publication failure boundaries. */
import assert from 'node:assert/strict';
import { generateKeyPairSync } from 'node:crypto';
import { mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { signDirectory, verifyDirectory } from '../native-release/manifest.mjs';
import { publishRelease, repositoryEndpoint } from '../native-release/publish.mjs';
import { releaseNotes } from '../native-release/prepare.mjs';
import { imagePlan } from '../native-release/containers.mjs';

const tag = 'v1.2.3';
const sha = 'a'.repeat(40);
const archive = 'rbit-1.2.3-x86_64-unknown-linux-gnu.tar.gz';
function keys() {
  return generateKeyPairSync('ed25519', { publicKeyEncoding: { type: 'spki', format: 'pem' }, privateKeyEncoding: { type: 'pkcs8', format: 'pem' } });
}
async function fixture(t) {
  const directory = await mkdtemp(join(tmpdir(), 'rbit-release-test-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const pair = keys();
  await writeFile(join(directory, archive), 'test archive bytes');
  await signDirectory(directory, tag, sha, pair.privateKey, pair.publicKey);
  return { directory, publicKey: pair.publicKey, tag, sha, token: 'test-only-token', origin: 'https://forge.example', repository: 'acme/rbit', notes: 'Release notes\n' };
}

test('signatures bind archives and exact source identity to a separately trusted key', async (t) => {
  const options = await fixture(t);
  const verified = await verifyDirectory(options.directory, options.publicKey, { tag, commit: sha });
  assert.equal(verified.entries.length, 4);
  await assert.rejects(verifyDirectory(options.directory, keys().publicKey), /signature/u);
  await assert.rejects(verifyDirectory(options.directory, options.publicKey, { tag, commit: 'b'.repeat(40) }), /identity/u);
  await writeFile(join(options.directory, archive), 'tampered');
  await assert.rejects(verifyDirectory(options.directory, options.publicKey), /checksum/u);
});

test('unlisted files, substituted metadata, symlinks and mismatched signing keys fail closed', async (t) => {
  const options = await fixture(t);
  await writeFile(join(options.directory, 'private-key.pem'), 'must never upload');
  await assert.rejects(verifyDirectory(options.directory, options.publicKey), /Unlisted/u);
  await rm(join(options.directory, 'private-key.pem'));
  await writeFile(join(options.directory, 'RELEASE.json'), '{}');
  await assert.rejects(verifyDirectory(options.directory, options.publicKey), /checksum/u);
  const pair = keys();
  await assert.rejects(signDirectory(options.directory, tag, sha, pair.privateKey, options.publicKey), /does not match/u);
  await rm(join(options.directory, 'RELEASE.json'));
  await rm(join(options.directory, archive));
  await symlink('RELEASE.json', join(options.directory, archive));
  await assert.rejects(signDirectory(options.directory, tag, sha, pair.privateKey, pair.publicKey), /Invalid release file/u);
});

function fakeApi() {
  const state = { release: null, assets: [], uploads: 0, calls: 0, failUpload: 0, moveAfterUpload: false, published: 0 };
  const json = (value, status = 200) => new Response(JSON.stringify(value), { status, headers: { 'Content-Type': 'application/json' } });
  const fetcher = async (url, options) => {
    state.calls += 1;
    assert.equal(new URL(url).origin, 'https://forge.example');
    assert.equal(options.headers.Authorization, 'Bearer test-only-token');
    assert.equal(options.redirect, 'error');
    const path = new URL(url).pathname.replace('/api/v1/repos/acme/rbit', '');
    if (path.startsWith('/commits/')) {
      assert.equal(decodeURIComponent(path.slice('/commits/'.length)), `refs/tags/${tag}`);
      return json({ sha: state.moveAfterUpload && state.uploads ? 'b'.repeat(40) : sha });
    }
    if (path.startsWith('/releases/tags/')) return state.release ? json(state.release) : json({}, 404);
    if (path === '/releases' && options.method === 'POST') {
      state.release = { ...JSON.parse(options.body), id: 1 };
      assert.equal(state.release.draft, true);
      return json(state.release, 201);
    }
    if (path === '/releases/1/assets' && options.method === 'GET') return json(state.assets.map(({ bytes, ...asset }) => asset));
    if (path === '/releases/1/assets' && options.method === 'POST') {
      state.uploads += 1;
      if (state.uploads === state.failUpload) return json({}, 503);
      const chunks = [];
      for await (const chunk of options.body) chunks.push(chunk);
      const bytes = Buffer.concat(chunks);
      const asset = { id: state.assets.length + 1, name: new URL(url).searchParams.get('name'), size: bytes.length, bytes };
      state.assets.push(asset);
      return json({ id: asset.id, name: asset.name, size: asset.size }, 201);
    }
    if (path.startsWith('/releases/assets/')) {
      const asset = state.assets.find((item) => item.id === Number(path.split('/').at(-1)));
      return new Response(asset.bytes);
    }
    if (path === '/releases/1' && options.method === 'PATCH') {
      assert.equal(state.assets.length, 4);
      state.published += 1;
      state.release.draft = false;
      return json(state.release);
    }
    throw new Error(`Unexpected fixture request ${path}`);
  };
  return { state, fetcher };
}

test('interrupted uploads remain drafts and resume after verifying existing bytes', async (t) => {
  const options = await fixture(t);
  const { state, fetcher } = fakeApi();
  state.failUpload = 2;
  await assert.rejects(publishRelease(options, fetcher), /503/u);
  assert.equal(state.release.draft, true);
  assert.equal(state.published, 0);
  const result = await publishRelease(options, fetcher);
  assert.deepEqual(result, { id: 1, tag, commit: sha });
  assert.equal(state.published, 1);
  const uploads = state.uploads;
  await publishRelease(options, fetcher);
  assert.equal(state.uploads, uploads);
  assert.equal(state.published, 1);
});

test('changed tags and corrupted remote bytes never publish or overwrite assets', async (t) => {
  const options = await fixture(t);
  const { state, fetcher } = fakeApi();
  state.moveAfterUpload = true;
  await assert.rejects(publishRelease(options, fetcher), /Remote tag/u);
  assert.equal(state.release.draft, true);
  state.moveAfterUpload = false;
  state.assets[0].bytes = Buffer.from('corrupted');
  await assert.rejects(publishRelease(options, fetcher), /asset.*(?:size|checksum)/u);
  assert.equal(state.published, 0);
});

test('local verification happens before any authenticated network call', async (t) => {
  const options = await fixture(t);
  const { state, fetcher } = fakeApi();
  const sums = await readFile(join(options.directory, 'SHA256SUMS'));
  await writeFile(join(options.directory, 'SHA256SUMS'), Buffer.concat([sums, Buffer.from('extra')]));
  await assert.rejects(publishRelease(options, fetcher), /signature/u);
  assert.equal(state.calls, 0);
});

test('origin and changelog validation reject ambiguous or unsafe publication inputs', () => {
  for (const origin of ['http://forge.example', 'https://user:secret@forge.example', 'https://forge.example/path', 'https://forge.example/?token=x']) {
    assert.throws(() => repositoryEndpoint(origin, 'acme/rbit'));
  }
  assert.throws(() => repositoryEndpoint('https://forge.example', '../rbit'));
  assert.equal(releaseNotes('## [Unreleased]\nPreview\n## [1.2.3]\nStable\n## [1.2.2]\nOlder\n', '1.2.3'), 'Stable\n');
  assert.equal(releaseNotes('## [Unreleased]\nPreview\n', '1.2.4-rc.1'), 'Preview\n');
  assert.throws(() => releaseNotes('## [Unreleased]\nPreview\n', '1.2.4'), /missing/u);
});

test('independent image plans keep prereleases off latest and never invoke a shell', () => {
  const plans = imagePlan('registry.example:5000', 'team/delivery', '1.2.3-rc.1', 'arm64', sha);
  assert.equal(plans.length, 2);
  assert.deepEqual(plans[0].tags, ['registry.example:5000/team/delivery/rbit:1.2.3-rc.1']);
  assert.ok(plans[0].arguments.includes('linux/arm64'));
  assert.ok(plans[1].arguments.includes('Dockerfile.runner'));
  assert.equal(imagePlan('registry.example', 'team', '1.2.3+build', 'amd64', sha)[0].tags[0], 'registry.example/team/rbit:1.2.3_build');
  assert.throws(() => imagePlan('https://registry.example', 'team', '1.2.3', 'amd64', sha));
  assert.throws(() => imagePlan('registry.example', 'team;command', '1.2.3', 'amd64', sha));
  assert.throws(() => imagePlan('registry.example', 'team', '1.2.3', 's390x', sha));
});


test('signing refuses pre-existing manifest symlinks without modifying their targets', async (t) => {
  const directory = await mkdtemp(join(tmpdir(), 'rbit-release-symlink-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const pair = keys();
  const target = join(directory, 'untouched');
  await writeFile(target, 'keep this file');
  await writeFile(join(directory, archive), 'archive');
  await symlink('untouched', join(directory, 'RELEASE.json'));
  await assert.rejects(signDirectory(directory, tag, sha, pair.privateKey, pair.publicKey), /EEXIST/u);
  assert.equal(await readFile(target, 'utf8'), 'keep this file');
});
