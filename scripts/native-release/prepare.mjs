/** Validate the release ref/version and extract bounded release notes before building. */
import { execFileSync } from 'node:child_process';
import { readFile, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { releaseIdentity } from './manifest.mjs';

export function releaseNotes(changelog, version) {
  const sections = [...changelog.matchAll(/^## \[([^\]]+)\][^\n]*\n([\s\S]*?)(?=^## \[|$(?![\s\S]))/gmu)];
  let notes = sections.find((section) => section[1] === version)?.[2]?.trim();
  if (!notes && version.split('+')[0].includes('-')) notes = sections.find((section) => section[1] === 'Unreleased')?.[2]?.trim();
  if (!notes || Buffer.byteLength(notes) > 64 * 1024) throw new Error('Versioned release notes are missing or too large');
  return `${notes}\n`;
}

export async function prepare(environment = process.env) {
  const tag = environment.RELEASE_TAG;
  const sha = environment.RELEASE_SHA;
  releaseIdentity(tag, sha);
  if (environment.GITHUB_REF !== `refs/tags/${tag}`) throw new Error('Dispatch the native release workflow on its version tag');
  const git = (...args) => execFileSync('git', args, { encoding: 'utf8', maxBuffer: 1024 * 1024 }).trim();
  if (git('rev-parse', 'HEAD') !== sha || git('rev-parse', `refs/tags/${tag}^{commit}`) !== sha) throw new Error('Checkout, tag and run SHA must agree');
  if (git('status', '--porcelain', '--untracked-files=no')) throw new Error('Release checkout is modified');
  const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--locked', '--no-deps', '--format-version', '1'], { encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 }));
  for (const name of ['rbit-server', 'rbit-runner']) {
    if (metadata.packages.find((item) => item.name === name)?.version !== tag.slice(1)) throw new Error('Tag and workspace version must agree');
  }
  const notes = releaseNotes(await readFile('CHANGELOG.md', 'utf8'), tag.slice(1));
  await writeFile('native-release-notes.md', notes);
  return { tag, commit: sha, version: tag.slice(1) };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  prepare().then((result) => console.log(JSON.stringify(result))).catch((error) => {
    console.error(`Release preparation failed: ${error.message}`);
    process.exitCode = 1;
  });
}
