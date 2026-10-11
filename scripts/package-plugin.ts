import { chmodSync, copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const platforms = new Set(['darwin-arm64', 'darwin-amd64', 'linux-amd64', 'linux-arm64', 'windows-amd64']);
const options = new Map<string, string>();
for (let index = 2; index < process.argv.length; index += 2) {
  const key = process.argv[index];
  const value = process.argv[index + 1];
  if (!['--binary', '--platform', '--version', '--output'].includes(key) || !value || options.has(key)) {
    throw new Error('Usage: node scripts/package-plugin.ts --binary PATH --platform PLATFORM --version VERSION --output NEW_DIRECTORY');
  }
  options.set(key, value);
}
const binary = options.get('--binary');
const platform = options.get('--platform');
const output = options.get('--output');
if (!binary || !platform || !output || !platforms.has(platform)) {
  throw new Error('Provide binary, one supported platform, a release version (e.g. 0.3.0), and output directory');
}
const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const staging = resolve(output);
if (existsSync(staging)) throw new Error(`Refusing to overwrite existing staging directory: ${staging}`);
const manifest = JSON.parse(readFileSync(join(repository, 'portable-package/plugin.json'), 'utf8')) as { version: string };
const version = options.get('--version') ?? manifest.version;
if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('Invalid release version');
if (manifest.version !== version) throw new Error('Release version must match portable-package/plugin.json');
cpSync(join(repository, 'portable-package'), staging, { recursive: true });
copyFileSync(join(repository, 'LICENSE'), join(staging, 'LICENSE'));
mkdirSync(join(staging, 'bin/schema'), { recursive: true });
const executable = platform === 'windows-amd64' ? 'agent-wait-on.exe' : 'agent-wait-on';
copyFileSync(resolve(binary), join(staging, 'bin', executable));
chmodSync(join(staging, 'bin', executable), 0o755);
copyFileSync(join(repository, 'schema/agent-exec.schema.json'), join(staging, 'bin/schema/agent-exec.schema.json'));
if (platform === 'windows-amd64') {
  const configPath = join(staging, 'mcp.json');
  const config = JSON.parse(readFileSync(configPath, 'utf8')) as { mcpServers: Record<string, { command: string }> };
  config.mcpServers['agent-wait-on'].command = './bin/agent-wait-on.exe';
  writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`);
}
console.log(staging);
