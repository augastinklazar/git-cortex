#!/usr/bin/env node

const { spawnSync, execSync } = require('child_process');
const path = require('path');
const fs = require('fs');
const os = require('os');
const crypto = require('crypto');
const https = require('https');
const pkg = require('../package.json');

function getBinaryName() {
  const platform = os.platform();
  const arch = os.arch();

  let ext = platform === 'win32' ? '.exe' : '';
  let target = '';
  let assetName = '';

  if (platform === 'linux') {
    if (arch === 'arm64') {
      target = 'aarch64-unknown-linux-gnu';
      assetName = 'git-cortex-aarch64-unknown-linux-gnu.tar.gz';
    } else {
      target = 'x86_64-unknown-linux-gnu';
      assetName = 'git-cortex-x86_64-unknown-linux-gnu.tar.gz';
    }
  } else if (platform === 'darwin') {
    if (arch === 'arm64') {
      target = 'aarch64-apple-darwin';
      assetName = 'git-cortex-aarch64-apple-darwin.tar.gz';
    } else {
      target = 'x86_64-apple-darwin';
      assetName = 'git-cortex-x86_64-apple-darwin.tar.gz';
    }
  } else if (platform === 'win32') {
    target = 'x86_64-pc-windows-msvc';
    assetName = 'git-cortex-x86_64-pc-windows-msvc.zip';
  } else {
    throw new Error(`Unsupported operating system platform: ${platform}`);
  }

  return { target, binaryName: `git-cortex${ext}`, assetName };
}

function verifyArchiveDigest(archiveBytes, expectedDigest) {
  const actualDigest = crypto.createHash('sha256').update(archiveBytes).digest('hex');
  if (actualDigest.toLowerCase() !== expectedDigest.toLowerCase()) {
    throw new Error(`Security error: SHA-256 digest mismatch! Expected ${expectedDigest}, got ${actualDigest}`);
  }
  return true;
}

function extractExpectedSha256(manifestText, assetName) {
  for (const line of manifestText.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    const parts = trimmed.split(/\s+/);
    if (parts.length >= 2 && parts[1].endsWith(assetName)) {
      return parts[0];
    }
  }
  return null;
}

function downloadBuffer(url) {
  return new Promise((resolve, reject) => {
    https.get(url, { headers: { 'User-Agent': 'git-cortex-installer' } }, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        return resolve(downloadBuffer(res.headers.location));
      }
      if (res.statusCode !== 200) {
        return reject(new Error(`Failed to download ${url}: HTTP status ${res.statusCode}`));
      }
      const chunks = [];
      res.on('data', chunk => chunks.push(chunk));
      res.on('end', () => resolve(Buffer.concat(chunks)));
      res.on('error', reject);
    }).on('error', reject);
  });
}

async function autoInstallBinary(targetInfo) {
  const userHome = os.homedir();
  const installDir = path.join(userHome, '.git-cortex', 'bin');
  const destPath = path.join(installDir, targetInfo.binaryName);

  if (fs.existsSync(destPath)) {
    return destPath;
  }

  const tag = `v${pkg.version}`;
  const baseUrl = `https://github.com/augastinklazar/git-cortex/releases/download/${tag}`;
  const manifestUrl = `${baseUrl}/SHA256SUMS.txt`;
  const archiveUrl = `${baseUrl}/${targetInfo.assetName}`;

  try {
    console.error(`git-cortex: Downloading release archive from ${archiveUrl}...`);
    const [manifestBuf, archiveBuf] = await Promise.all([
      downloadBuffer(manifestUrl),
      downloadBuffer(archiveUrl),
    ]);

    const manifestText = manifestBuf.toString('utf8');
    const expectedSha256 = extractExpectedSha256(manifestText, targetInfo.assetName);
    if (!expectedSha256) {
      throw new Error(`Asset '${targetInfo.assetName}' not found in SHA256SUMS.txt manifest`);
    }

    verifyArchiveDigest(archiveBuf, expectedSha256);
    console.error(`git-cortex: Cryptographic SHA-256 integrity verified (${expectedSha256}).`);

    fs.mkdirSync(installDir, { recursive: true });
    const tmpArchive = path.join(installDir, `tmp_${targetInfo.assetName}`);
    fs.writeFileSync(tmpArchive, archiveBuf);

    if (targetInfo.assetName.endsWith('.tar.gz')) {
      execSync(`tar -xzf "${tmpArchive}" -C "${installDir}"`, { stdio: 'pipe' });
    } else if (targetInfo.assetName.endsWith('.zip')) {
      if (process.platform === 'win32') {
        execSync(`powershell -NoProfile -Command "Expand-Archive -Path '${tmpArchive}' -DestinationPath '${installDir}' -Force"`, { stdio: 'pipe' });
      } else {
        execSync(`unzip -o "${tmpArchive}" -d "${installDir}"`, { stdio: 'pipe' });
      }
    }
    try { fs.unlinkSync(tmpArchive); } catch (_) {}

    if (fs.existsSync(destPath)) {
      try { fs.chmodSync(destPath, 0o755); } catch (_) {}
      return destPath;
    }
  } catch (err) {
    console.error(`git-cortex: Automated release download or checksum verification failed: ${err.message}`);
  }
  return null;
}

function findOnPathExcludingSelf(binaryName) {
  const pathEnv = process.env.PATH || '';
  const pathSep = process.platform === 'win32' ? ';' : ':';
  let selfReal = '';
  try {
    selfReal = fs.realpathSync(__filename);
  } catch (_) {
    selfReal = __filename;
  }

  for (const dir of pathEnv.split(pathSep)) {
    if (!dir) continue;
    const candidate = path.join(dir, binaryName);
    if (fs.existsSync(candidate)) {
      try {
        if (fs.realpathSync(candidate) !== selfReal) {
          return candidate;
        }
      } catch (_) {}
    }
  }
  return null;
}

async function resolveBinary() {
  const targetInfo = getBinaryName();
  const { target, binaryName } = targetInfo;

  // 1. Direct environment override
  if (process.env.GIT_CORTEX_BIN && fs.existsSync(process.env.GIT_CORTEX_BIN)) {
    return process.env.GIT_CORTEX_BIN;
  }

  // 2. Pre-packaged platform directory
  const localPkgBin = path.join(__dirname, '..', 'binaries', target, binaryName);
  if (fs.existsSync(localPkgBin)) {
    return localPkgBin;
  }

  // 3. User home installation directory (~/.git-cortex/bin)
  const homeBin = path.join(os.homedir(), '.git-cortex', 'bin', binaryName);
  if (fs.existsSync(homeBin)) {
    return homeBin;
  }

  // 4. Fallback to cargo release target if in dev workspace
  const devBin = path.join(__dirname, '..', '..', 'target', 'release', binaryName);
  if (fs.existsSync(devBin)) {
    return devBin;
  }

  // 5. Fallback to system PATH excluding this wrapper script
  const onPath = findOnPathExcludingSelf(binaryName);
  if (onPath) {
    return onPath;
  }

  // 6. Automated verified download from GitHub Releases
  const installedBin = await autoInstallBinary(targetInfo);
  if (installedBin) {
    return installedBin;
  }

  return null;
}

async function main() {
  const binaryPath = await resolveBinary();
  if (!binaryPath) {
    const { binaryName } = getBinaryName();
    console.error(`git-cortex native executable '${binaryName}' was not found.`);
    console.error('Please install git-cortex via cargo or download a release binary:');
    console.error('  cargo install --git https://github.com/augastinklazar/git-cortex cortex-cli');
    process.exit(1);
  }

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
}

if (require.main === module) {
  main();
}

module.exports = {
  getBinaryName,
  verifyArchiveDigest,
  extractExpectedSha256,
};
