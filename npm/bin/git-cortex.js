#!/usr/bin/env node

const { spawnSync } = require('child_process');
const path = require('path');
const fs = require('fs');
const os = require('os');

function getBinaryName() {
  const platform = os.platform();
  const arch = os.arch();

  let ext = platform === 'win32' ? '.exe' : '';
  let target = '';

  if (platform === 'linux') {
    target = arch === 'arm64' ? 'linux-arm64' : 'linux-x64';
  } else if (platform === 'darwin') {
    target = arch === 'arm64' ? 'darwin-arm64' : 'darwin-x64';
  } else if (platform === 'win32') {
    target = arch === 'arm64' ? 'win32-arm64' : 'win32-x64';
  } else {
    throw new Error(`Unsupported operating system platform: ${platform}`);
  }

  return { target, binaryName: `git-cortex${ext}` };
}

function resolveBinary() {
  const { target, binaryName } = getBinaryName();

  // 1. Direct environment override
  if (process.env.GIT_CORTEX_BIN && fs.existsSync(process.env.GIT_CORTEX_BIN)) {
    return process.env.GIT_CORTEX_BIN;
  }

  // 2. Pre-packaged platform directory
  const localPkgBin = path.join(__dirname, '..', 'binaries', target, binaryName);
  if (fs.existsSync(localPkgBin)) {
    return localPkgBin;
  }

  // 3. Fallback to cargo release target if in dev workspace
  const devBin = path.join(__dirname, '..', '..', 'target', 'release', binaryName);
  if (fs.existsSync(devBin)) {
    return devBin;
  }

  // 4. Fallback to system PATH
  return binaryName;
}

const binaryPath = resolveBinary();
const args = process.argv.slice(2);

const result = spawnSync(binaryPath, args, {
  stdio: 'inherit',
  shell: false,
  env: process.env,
});

if (result.error) {
  if (result.error.code === 'ENOENT') {
    console.error(`git-cortex binary not found at ${binaryPath}.`);
    console.error('Please install git-cortex via cargo or download a release binary:');
    console.error('  cargo install --git https://github.com/augastinklazar/git-cortex cortex-cli');
  } else {
    console.error(`Failed to execute git-cortex: ${result.error.message}`);
  }
  process.exit(1);
}

process.exit(result.status ?? 0);
