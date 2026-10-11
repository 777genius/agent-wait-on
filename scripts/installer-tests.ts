// Fixture checks: a corrupt download/cache must never reach the client installer;
// replay must retain a stable, unmodified package source. All profiles are disposable.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const windows = process.platform === 'win32';
const platform = windows ? 'windows-amd64' : `${process.platform}-${process.arch === 'x64' ? 'amd64' : 'arm64'}`;
const fixture = mkdtempSync(join(tmpdir(), 'agent-wait-on-installer-'));
const release = join(fixture, 'release');
const helpers = join(fixture, 'helpers');
mkdirSync(release);
mkdirSync(helpers);
const binary = join(fixture, windows ? 'fixture.exe' : 'fixture-binary');
writeFileSync(binary, 'fixture-only binary; never execute\n');
const staged = join(fixture, 'staged');

function execute(command: string, args: string[], options: { cwd?: string; env?: NodeJS.ProcessEnv } = {}) {
  return spawnSync(command, args, { encoding: 'utf8', ...options });
}
function requireSuccess(result: ReturnType<typeof execute>) {
  assert.equal(result.status, 0, result.stderr || result.stdout || result.error?.message || 'process failed');
}
function quotedPowerShell(value: string) { return `'${value.replaceAll("'", "''")}'`; }

try {
  requireSuccess(execute(process.execPath, ['--experimental-strip-types', join(repository, 'scripts/package-plugin.ts'),
    '--binary', binary, '--platform', platform, '--output', staged]));
  assert.equal(JSON.parse(readFileSync(join(staged, 'plugin.json'), 'utf8')).version, '0.3.0');
  assert.equal(JSON.parse(readFileSync(join(staged, 'mcp.json'), 'utf8')).mcpServers['agent-wait-on'].command,
    windows ? './bin/agent-wait-on.exe' : './bin/agent-wait-on');
  assert.equal(readFileSync(join(staged, 'bin/schema/agent-exec.schema.json'), 'utf8'),
    readFileSync(join(repository, 'schema/agent-exec.schema.json'), 'utf8'));
  const asset = `agent-wait-on-v0.3.0-${platform}.zip`;
  const archive = join(release, asset);
  if (windows) {
    requireSuccess(execute('pwsh', ['-NoProfile', '-Command',
      `Compress-Archive -Path ${quotedPowerShell(join(staged, '*'))} -DestinationPath ${quotedPowerShell(archive)}`]));
  } else {
    requireSuccess(execute('zip', ['-q', '-r', archive, '.'], { cwd: staged }));
  }
  const checksum = createHash('sha256').update(readFileSync(archive)).digest('hex');
  const sums = join(release, 'SHA256SUMS');
  writeFileSync(sums, `${checksum}  ${asset}\n`);
  if (windows) {
    writeFileSync(join(helpers, 'agentplugins.ps1'), `
Add-Content -LiteralPath $env:FIXTURE_UAP_LOG -Value ($args | ConvertTo-Json -Compress)
exit 0
`);
  } else {
    writeFileSync(join(helpers, 'curl'), `#!/bin/sh
set -eu
name= output=
while [ "$#" -gt 0 ]; do
  case "$1" in
    https://github.com/777genius/agent-wait-on/releases/download/v0.3.0/*) name=$(basename "$1") ;;
    --output) shift; output=$1 ;;
  esac
  shift
done
[ -n "$name" ] && [ -n "$output" ]
cp "$FIXTURE_RELEASE/$name" "$output"
`);
    writeFileSync(join(helpers, 'agentplugins'), `#!/bin/sh
set -eu
printf '%s\\n' "$*" >> "$FIXTURE_UAP_LOG"
`);
    chmodSync(join(helpers, 'curl'), 0o755);
    chmodSync(join(helpers, 'agentplugins'), 0o755);
  }
  function profile(name: string): NodeJS.ProcessEnv {
    const home = join(fixture, name);
    mkdirSync(home);
    return { ...process.env, HOME: home, USERPROFILE: home, CODEX_HOME: join(home, '.codex'),
      CLAUDE_CONFIG_DIR: join(home, '.claude'), AGENTPLUGINS_HOME: join(home, '.agentplugins'),
      XDG_CACHE_HOME: join(home, 'cache'), LOCALAPPDATA: join(home, 'localappdata'),
      PATH: `${helpers}${windows ? ';' : ':'}${process.env.PATH ?? ''}`,
      FIXTURE_RELEASE: release, FIXTURE_UAP_LOG: join(home, 'uap.log') };
  }
  function install(env: NodeJS.ProcessEnv, target = true) {
    if (windows) {
      // Shadow only download I/O. The fixture helper never touches a real client.
      return execute('pwsh', ['-NoProfile', '-Command', `
function Invoke-WebRequest { param([string]$Uri, [string]$OutFile)
  if (-not $Uri.StartsWith('https://github.com/777genius/agent-wait-on/releases/download/v0.3.0/')) { throw 'Unexpected source' }
  Copy-Item -LiteralPath (Join-Path $env:FIXTURE_RELEASE ($Uri.Split('/')[-1])) -Destination $OutFile
}
& ${quotedPowerShell(join(repository, 'install.ps1'))}${target ? " --target 'codex,claude'" : ''}
`], { env });
    }
    return execute('sh', [join(repository, 'install.sh'), ...(target ? ['--target', 'codex,claude'] : [])], { env });
  }
  const valid = profile('valid');
  requireSuccess(install(valid));
  const source = windows ? join(valid.LOCALAPPDATA!, 'agent-wait-on/releases/v0.3.0/windows-amd64/package')
    : join(valid.XDG_CACHE_HOME!, `agent-wait-on/releases/v0.3.0/${platform}/package`);
  const manifest = join(source, 'plugin.json');
  const initialModified = statSync(manifest).mtimeMs;
  requireSuccess(install(valid, false));
  assert.equal(statSync(manifest).mtimeMs, initialModified, 'repeat install must not replace source package');
  const calls = readFileSync(valid.FIXTURE_UAP_LOG!, 'utf8').trim().split(/\r?\n/);
  assert.equal(calls.length, 4, 'each invocation validates then delegates add');
  assert.match(calls[0], /validate/);
  assert.match(calls[1], /codex,claude/);
  assert.match(calls[2], /validate/);
  assert.doesNotMatch(calls[3], /--target/, 'default delegates discovery to UAP');
  writeFileSync(manifest, 'tampered cached source');
  assert.notEqual(install(valid).status, 0, 'changed persistent source must be rejected');
  assert.equal(readFileSync(valid.FIXTURE_UAP_LOG!, 'utf8').trim().split(/\r?\n/).length, 4);
  writeFileSync(join(dirname(source), asset), 'tampered cached archive');
  const damagedCache = install(valid);
  assert.notEqual(damagedCache.status, 0, 'cached archive must be rehashed');
  assert.match(damagedCache.stderr, /Cached archive SHA256 verification failed/);
  assert.equal(readFileSync(valid.FIXTURE_UAP_LOG!, 'utf8').trim().split(/\r?\n/).length, 4);
  writeFileSync(sums, `${'0'.repeat(64)}  ${asset}\n`);
  const corrupt = profile('corrupt');
  assert.notEqual(install(corrupt).status, 0, 'checksum mismatch must fail');
  assert.equal(existsSync(corrupt.FIXTURE_UAP_LOG!), false, 'checksum failure must not invoke UAP');
  console.log(`PASS: ${platform} package, repeat/discovery, immutable source, checksum failure; temporary profiles only`);
} finally {
  rmSync(fixture, { recursive: true, force: true });
}
