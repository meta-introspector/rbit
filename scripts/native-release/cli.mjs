#!/usr/bin/env node
/** Native release signing, verification and publication; credentials come only from environment. */
import { readFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { signDirectory, verifyDirectory } from './manifest.mjs';
import { publishRelease } from './publish.mjs';

export async function main(args, environment = process.env) {
  const [command, directory, keyPath] = args;
  if (!directory || args.length > 3) throw new Error('Usage: node scripts/native-release/cli.mjs sign|verify|publish DIRECTORY [PUBLIC_KEY_FILE]');
  const publicKey = keyPath ? await readFile(keyPath, 'utf8') : environment.RELEASE_PUBLIC_KEY;
  if (!publicKey) throw new Error('Provide a trusted public key separately from the release download');
  if (command === 'verify') return verifyDirectory(directory, publicKey);
  const tag = environment.RELEASE_TAG;
  const sha = environment.RELEASE_SHA;
  if (command === 'sign') {
    if (!environment.RELEASE_SIGNING_KEY) throw new Error('RELEASE_SIGNING_KEY is required');
    return signDirectory(directory, tag, sha, environment.RELEASE_SIGNING_KEY, publicKey);
  }
  if (command !== 'publish') throw new Error('Unknown native release command');
  if (!environment.RELEASE_NOTES_FILE) throw new Error('RELEASE_NOTES_FILE is required');
  return publishRelease({
    directory, publicKey, tag, sha,
    origin: environment.RELEASE_ORIGIN,
    repository: environment.RELEASE_REPOSITORY,
    token: environment.RELEASE_TOKEN,
    notes: await readFile(environment.RELEASE_NOTES_FILE, 'utf8'),
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main(process.argv.slice(2)).then((result) => {
    console.log(JSON.stringify(result));
  }).catch((error) => {
    console.error(`Native release failed: ${error.message}`);
    process.exitCode = 1;
  });
}
