import type {} from 'node:process';

const assert: typeof import('node:assert/strict') = require('node:assert/strict');
const fs = require('node:fs/promises') as typeof import('node:fs/promises');
const os = require('node:os') as typeof import('node:os');
const path = require('node:path') as typeof import('node:path');
const { spawn, spawnSync } = require('node:child_process') as typeof import('node:child_process');
const { once } = require('node:events') as typeof import('node:events');
const { UUID } = require('builder-util-runtime') as typeof import('builder-util-runtime');

function run(file: string, args: string[], env: NodeJS.ProcessEnv): Promise<number | null> {
  return new Promise((resolve, reject) => {
    const child = spawn(file, args, {
      argv0: `"${file}"`,
      env,
      stdio: 'ignore',
      windowsHide: true,
      windowsVerbatimArguments: true,
    });
    const timeout = setTimeout(() => {
      child.kill();
      reject(new Error(`Timed out running ${path.basename(file)} ${args.join(' ')}`));
    }, 600_000);
    child.once('error', error => {
      clearTimeout(timeout);
      reject(error);
    });
    child.once('close', code => {
      clearTimeout(timeout);
      resolve(code);
    });
  });
}

function existsInRegistry(key: string, view: '32' | '64'): boolean {
  const result = spawnSync('reg.exe', ['query', key, `/reg:${view}`], {
    stdio: 'ignore',
    windowsHide: true,
  });
  if (result.error || (result.status !== 0 && result.status !== 1)) {
    throw new Error('Could not check the existing Refract registration', { cause: result.error });
  }
  return result.status === 0;
}

async function checkRunningGuard(
  fixture: string,
  verify: () => Promise<void>,
): Promise<void> {
  const child = spawn(fixture, ['-t', '127.0.0.1'], { stdio: 'ignore', windowsHide: true });
  await once(child, 'spawn');
  const exited = once(child, 'exit');
  try {
    await verify();
    assert.equal(child.exitCode, null, 'Refract Setup must leave the running process alone');
  } finally {
    child.kill();
    await exited;
  }
}

async function verifyInstaller(output: string, mode?: string, electronOutput?: string): Promise<void> {
  if (process.platform !== 'win32' || process.env.GITHUB_ACTIONS !== 'true'
    || process.env.RUNNER_ENVIRONMENT !== 'github-hosted') {
    throw new Error('The lifecycle test requires a disposable GitHub-hosted Windows runner.');
  }
  const releaseAssets = mode === '--release-assets';
  if (!output || (mode !== undefined && !releaseAssets)) {
    throw new Error('Pass the native installer output directory, followed by optional --release-assets.');
  }
  const { APPDATA: appData, SystemRoot: systemRoot } = process.env;
  if (!appData || !systemRoot) throw new Error('The Windows runner did not provide APPDATA and SystemRoot.');

  const manifest = JSON.parse(await fs.readFile(path.resolve(__dirname, '../../package.json'), 'utf8')) as {
    version: string;
    main: string;
    build: { appId: string; asar?: boolean; productName: string; nsis: { guid?: string } };
  };
  const { version, build: { appId, productName, nsis } } = manifest;
  const guid = nsis.guid ?? UUID.v5(appId, UUID.parse('50e065bc-3134-11e6-9bab-38c9862bdaf3'));
  const installKey = `Software\\${guid}`;
  const uninstallKey = `Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\${guid}`;
  for (const hive of ['HKCU', 'HKLM']) {
    for (const view of ['32', '64'] as const) {
      for (const key of [installKey, uninstallKey]) {
        assert.equal(existsInRegistry(`${hive}\\${key}`, view), false, 'Refract is already registered');
      }
    }
  }
  function verifyRegistration(present: boolean): void {
    for (const key of [installKey, uninstallKey]) {
      assert.equal(existsInRegistry(`HKCU\\${key}`, '64'), present);
    }
  }

  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'refract-native-installer-'));
  const installDir = path.join(root, 'Installed Refract');
  const localAppData = path.join(root, 'local');
  await fs.mkdir(localAppData);
  const env = { ...process.env, LOCALAPPDATA: localAppData };
  const project = path.join(appData, productName, 'projects', 'installer-fixture.tex');
  await assert.rejects(fs.access(path.join(appData, productName)), { code: 'ENOENT' });
  await fs.mkdir(path.dirname(project), { recursive: true });
  await fs.writeFile(project, 'Fixture project');

  const installer = path.resolve(output, `${productName}${releaseAssets ? '' : '-Native'}-Setup-${version}.exe`);
  const executable = path.join(installDir, `${productName}.exe`);
  const fontLicense = path.join(installDir, 'Inter-OFL.txt');
  const installedVersion = path.join(installDir, 'refract-native-installed');
  const uninstaller = path.join(installDir, `Uninstall ${productName}.exe`);
  const uninstallerCopy = path.join(root, 'uninstaller-copy.exe');
  const firstInstall = ['/S', `/D=${installDir}`];

  console.log('Installing Refract and setting up the managed TeX toolchain');
  assert.equal(await run(installer, firstInstall, env), 0);
  verifyRegistration(true);
  await fs.access(executable);
  await fs.access(uninstaller);
  assert.equal(await fs.readFile(fontLicense, 'utf8'),
    await fs.readFile(path.resolve(__dirname, '../../resources/fonts/Inter-OFL.txt'), 'utf8'));
  assert.equal((await fs.readFile(installedVersion, 'utf8')).trim(), version);
  const dependencies = path.join(localAppData, productName, 'dependencies');
  const bin = path.join(dependencies, 'TinyTeX', 'bin', 'windows');
  for (const tool of ['pdflatex', 'xelatex', 'lualatex', 'kpsewhich', 'synctex', 'bibtex', 'biber']) {
    assert.ok((await fs.stat(path.join(bin, `${tool}.exe`))).size > 1000, `Installed ${tool}.exe`);
  }
  const texMarker = path.join(bin, 'keep-existing-tex.txt');
  await fs.writeFile(texMarker, 'Existing toolchain');

  console.log('Checking refusal to install or uninstall while Refract is running');
  const blocker = path.join(root, `${productName}.exe`);
  await fs.copyFile(path.join(systemRoot, 'System32', 'ping.exe'), blocker);
  await fs.copyFile(uninstaller, uninstallerCopy);
  const uninstallArgs = ['/S', `_?=${installDir}`];
  await checkRunningGuard(blocker, async () => {
    assert.equal(await run(installer, ['/S'], env), 2);
    assert.equal(await run(uninstallerCopy, uninstallArgs, env), 2);
  });

  console.log('Checking that a TeX setup failure leaves the existing installation intact');
  const failedLocalAppData = path.join(root, 'failed');
  const failedDependencies = path.join(failedLocalAppData, productName, 'dependencies');
  await fs.mkdir(path.join(failedDependencies, 'setup.lock'), { recursive: true });
  assert.equal(await run(installer, ['/S', '--updated'], {
    ...env,
    LOCALAPPDATA: failedLocalAppData,
  }), 3);
  assert.ok((await fs.readFile(path.join(failedDependencies, 'setup-error.log'), 'utf8')).trim());
  verifyRegistration(true);
  await fs.access(executable);

  console.log('Checking upgrade, managed TeX reuse, and owned-file cleanup');
  const unrelated = path.join(installDir, 'preserve.txt');
  await fs.writeFile(unrelated, 'Not owned by installer');
  assert.equal(await run(installer, ['/S', '--updated'], env), 0);
  assert.equal((await fs.readFile(installedVersion, 'utf8')).trim(), version);
  assert.equal(await fs.readFile(texMarker, 'utf8'), 'Existing toolchain');
  await fs.copyFile(uninstaller, uninstallerCopy);
  assert.equal(await run(uninstallerCopy, ['/S', '--delete-app-data', `_?=${installDir}`], env), 2);
  assert.equal(await run(uninstallerCopy, uninstallArgs, env), 0);
  verifyRegistration(false);
  for (const file of [executable, fontLicense, installedVersion, uninstaller]) {
    await assert.rejects(fs.access(file), { code: 'ENOENT' });
  }
  assert.equal(await fs.readFile(project, 'utf8'), 'Fixture project');
  assert.equal(await fs.readFile(unrelated, 'utf8'), 'Not owned by installer');
  assert.equal(await fs.readFile(texMarker, 'utf8'), 'Existing toolchain');

  if (electronOutput) {
    console.log('Upgrading the existing Electron installer to the native release asset');
    const electronInstaller = path.resolve(electronOutput, `${productName} Setup ${version}.exe`);
    const previousInstall = path.join(root, 'Installed Electron');
    const electronApp = manifest.build.asar === false
      ? path.join(previousInstall, 'resources', 'app', manifest.main)
      : path.join(previousInstall, 'resources', 'app.asar');
    const updateTrack = path.join(appData, productName, 'update-settings.json');
    await fs.writeFile(updateTrack, '{"track":"beta"}\n');
    assert.equal(await run(electronInstaller, ['/S', `/D=${previousInstall}`], env), 0);
    verifyRegistration(true);
    await fs.access(electronApp);

    assert.equal(await run(installer, ['/S', '--updated'], env), 0);
    verifyRegistration(true);
    await fs.access(path.join(previousInstall, `${productName}.exe`));
    assert.equal((await fs.readFile(path.join(previousInstall, 'refract-native-installed'), 'utf8')).trim(), version);
    await assert.rejects(fs.access(electronApp), { code: 'ENOENT' });
    assert.equal(await fs.readFile(project, 'utf8'), 'Fixture project');
    assert.equal(await fs.readFile(updateTrack, 'utf8'), '{"track":"beta"}\n');
    assert.equal(await fs.readFile(texMarker, 'utf8'), 'Existing toolchain');

    await fs.copyFile(path.join(previousInstall, `Uninstall ${productName}.exe`), uninstallerCopy);
    assert.equal(await run(uninstallerCopy, ['/S', `_?=${previousInstall}`], env), 0);
    verifyRegistration(false);
    assert.equal(await fs.readFile(project, 'utf8'), 'Fixture project');
    assert.equal(await fs.readFile(updateTrack, 'utf8'), '{"track":"beta"}\n');
    assert.equal(await fs.readFile(texMarker, 'utf8'), 'Existing toolchain');
  }
}

verifyInstaller(process.argv[2], process.argv[3], process.argv[4]).catch((error: unknown) => {
  console.error(error);
  process.exitCode = 1;
});
