/** Publish native Linux containers from verified release binaries to an explicit registry. */
import { execFileSync } from 'node:child_process';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { verifyDirectory } from './manifest.mjs';

export function imagePlan(registry, namespace, version, arch, sha) {
  if (!/^[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?(?::[1-9][0-9]{0,4})?$/u.test(registry ?? '') || registry.length > 253) throw new Error('Set an explicit registry hostname, without a URL scheme');
  if (!/^[a-z0-9]+(?:[._-][a-z0-9]+)*(?:\/[a-z0-9]+(?:[._-][a-z0-9]+)*)*$/u.test(namespace ?? '') || namespace.length > 128) throw new Error('Invalid container namespace');
  if (!['amd64', 'arm64'].includes(arch)) throw new Error('Only native Linux amd64 and arm64 containers are supported');
  if (!/^[0-9A-Za-z][0-9A-Za-z.+-]{0,126}$/u.test(version) || !/^[a-f0-9]{40}$/u.test(sha)) throw new Error('Invalid image release identity');
  return ['rbit', 'rbit-runner'].map((binary) => {
    const image = `${registry}/${namespace}/${binary}`;
    const tags = [`${image}:${version.replaceAll('+', '_')}`];
    if (!version.split('+')[0].includes('-')) tags.push(`${image}:latest`);
    const arguments_ = ['buildx', 'build', '--platform', `linux/${arch}`, '--build-arg', 'BINARY=prebuilt', '--file', binary === 'rbit' ? 'Dockerfile' : 'Dockerfile.runner'];
    for (const tag of tags) arguments_.push('--tag', tag);
    arguments_.push('--label', `org.opencontainers.image.version=${version}`, '--label', `org.opencontainers.image.revision=${sha}`, '--provenance=mode=max', '--sbom=true', '--push', '.');
    return { binary, image, tags, arguments: arguments_ };
  });
}

export async function publishContainers(directory, environment = process.env) {
  if (process.platform !== 'linux') throw new Error('Container publication requires a native Linux runner');
  const verified = await verifyDirectory(directory, environment.RELEASE_PUBLIC_KEY, { tag: environment.RELEASE_TAG, commit: environment.RELEASE_SHA });
  const arch = { x64: 'amd64', arm64: 'arm64' }[process.arch];
  const target = { amd64: 'x86_64-unknown-linux-gnu', arm64: 'aarch64-unknown-linux-gnu' }[arch];
  const version = verified.identity.tag.slice(1);
  const plans = imagePlan(environment.RELEASE_REGISTRY, environment.RELEASE_NAMESPACE, version, arch, verified.identity.commit);
  const archiveName = `rbit-${version}-${target}.tar.gz`;
  if (!verified.entries.some(({ name }) => name === archiveName)) throw new Error('No signed archive for this runner architecture');
  if (!environment.RELEASE_REGISTRY_USER || !environment.RELEASE_REGISTRY_PASSWORD) throw new Error('Registry credentials are required');
  const output = resolve('dist', `linux-${arch}`);
  await mkdir(output, { recursive: true });
  const dockerConfig = await mkdtemp(join(tmpdir(), 'rbit-release-docker-'));
  const created = [];
  try {
    for (const { binary } of plans) {
      const bytes = execFileSync('tar', ['-xOf', join(directory, archiveName), `rbit-${version}-${target}/${binary}`], { maxBuffer: 256 * 1024 * 1024 });
      // Extract to stdout, then create a fixed regular filename: archive paths/links never write to disk.
      await writeFile(join(output, binary), bytes, { mode: 0o755, flag: 'wx' });
      created.push(binary);
    }
    const options = { env: { ...process.env, DOCKER_CONFIG: dockerConfig }, maxBuffer: 1024 * 1024 };
    execFileSync('docker', ['login', environment.RELEASE_REGISTRY, '--username', environment.RELEASE_REGISTRY_USER, '--password-stdin'], { ...options, input: `${environment.RELEASE_REGISTRY_PASSWORD}\n`, stdio: ['pipe', 'ignore', 'pipe'] });
    for (const plan of plans) execFileSync('docker', plan.arguments, { ...options, stdio: ['ignore', 'inherit', 'inherit'] });
    return plans.map(({ binary, tags }) => ({ binary, tags, commit: verified.identity.commit }));
  } finally {
    await rm(dockerConfig, { recursive: true, force: true });
    for (const binary of created) await rm(join(output, binary), { force: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  publishContainers(process.argv[2]).then((result) => console.log(JSON.stringify(result))).catch(() => {
    // Child-process errors may contain registry output; never echo credential-bearing error objects.
    console.error('Container publication failed; check the validated registry settings and build output');
    process.exitCode = 1;
  });
}
