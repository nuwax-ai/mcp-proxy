#!/usr/bin/env node
'use strict';

const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

function platformKey() {
  if (process.platform === 'darwin' && process.arch === 'arm64') {
    return 'darwin-arm64';
  }
  if (process.platform === 'darwin' && process.arch === 'x64') {
    return 'darwin-x64';
  }
  if (process.platform === 'linux' && process.arch === 'x64') {
    return 'linux-x64';
  }
  if (process.platform === 'win32' && process.arch === 'x64') {
    return 'windows-x64';
  }
  if (process.platform === 'linux' && process.arch === 'arm64') {
    return 'linux-arm64';
  }
  return null;
}

function main() {
  const key = platformKey();
  if (!key) {
    console.error(
      `nuwax-deploy-installer: unsupported platform ${process.platform}-${process.arch}`
    );
    process.exit(1);
  }

  const pkgRoot = path.resolve(__dirname, '..');
  const vendorRoot = path.join(pkgRoot, 'vendor');
  const exeSuffix = process.platform === 'win32' ? '.exe' : '';
  const binary = path.join(vendorRoot, key, `deploy-installer${exeSuffix}`);

  if (!fs.existsSync(binary)) {
    console.error(
      `nuwax-deploy-installer: missing binary for ${key} at ${binary}\n` +
        'Reinstall the package or use a build that includes your platform.'
    );
    process.exit(1);
  }

  const env = {
    ...process.env,
    NUWAX_DEPLOY_ROOT: vendorRoot,
    NUWAX_DEPLOY_VERSION: require(path.join(pkgRoot, 'package.json')).version,
  };

  const result = spawnSync(binary, process.argv.slice(2), {
    stdio: 'inherit',
    env,
  });

  if (result.error) {
    console.error(result.error.message);
    process.exit(1);
  }
  process.exit(result.status ?? 1);
}

main();
