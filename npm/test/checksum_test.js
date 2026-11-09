const assert = require('assert');
const crypto = require('crypto');
const { verifyArchiveDigest, extractExpectedSha256 } = require('../bin/git-cortex.js');

// Test 1: verifyArchiveDigest passes on matching digest
const sampleData = Buffer.from('cortex test payload');
const expectedHex = crypto.createHash('sha256').update(sampleData).digest('hex');
assert.strictEqual(verifyArchiveDigest(sampleData, expectedHex), true);

// Test 2: verifyArchiveDigest throws on mismatch
assert.throws(() => {
  verifyArchiveDigest(sampleData, 'deadbeef00000000000000000000000000000000000000000000000000000000');
}, /Security error: SHA-256 digest mismatch/);

// Test 3: extractExpectedSha256 parses SHA256SUMS.txt correctly
const manifest = `
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  git-cortex-x86_64-unknown-linux-gnu.tar.gz
1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef  git-cortex-x86_64-pc-windows-msvc.zip
`;
const extracted = extractExpectedSha256(manifest, 'git-cortex-x86_64-pc-windows-msvc.zip');
assert.strictEqual(extracted, '1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef');

console.log('All checksum verification tests passed.');
