import type { Configuration } from 'electron-builder';

const { createHash } = require('node:crypto') as typeof import('node:crypto');
const { createReadStream } = require('node:fs') as typeof import('node:fs');
const fs = require('node:fs/promises') as typeof import('node:fs/promises');
const os = require('node:os') as typeof import('node:os');
const path = require('node:path') as typeof import('node:path');
const { build, createTargets, Platform } = require('electron-builder') as typeof import('electron-builder');
const { editWindowsResources } = require('app-builder-lib/out/util/resEdit') as typeof import('app-builder-lib/out/util/resEdit');
const { parseUpdateInfo } = require('electron-updater/out/providers/Provider') as typeof import('electron-updater/out/providers/Provider');
const semver = require('semver') as typeof import('semver');

async function packageWindows([app, dependencyHelper, output, mode]: string[]): Promise<void> {
  const releaseAssets = mode === '--release-assets';
  if (process.platform !== 'win32' || process.arch !== 'x64' || !app || !dependencyHelper || !output
    || (mode !== undefined && !releaseAssets)) {
    throw new Error('On x64 Windows, pass the native application, dependency helper, and installer output directory, followed by optional --release-assets.');
  }

  const root = path.resolve(__dirname, '../..');
  const manifest = JSON.parse(await fs.readFile(path.join(root, 'package.json'), 'utf8')) as {
    version: string;
    build: Configuration;
  };
  const product = manifest.build.productName;
  if (!product) throw new Error('Missing productName in package.json.');
  const version = manifest.version;
  const channel = semver.prerelease(version) ? 'beta' : 'latest';
  const publishOptions = [manifest.build.publish].flat();
  const github = publishOptions.find(option => typeof option === 'object' && option?.provider === 'github');
  if (releaseAssets && (!github || typeof github !== 'object')) {
    throw new Error('Missing GitHub update configuration in package.json.');
  }
  const outputDir = path.resolve(output);
  const executableName = `${product}.exe`;
  const iconPath = path.join(root, 'resources/logo.ico');
  const temp = await fs.realpath(os.tmpdir());
  const stage = await fs.mkdtemp(path.join(temp, 'refract-native-payload-'));
  try {
    const executable = path.join(stage, executableName);
    await fs.copyFile(path.resolve(app), executable);
    await fs.copyFile(path.join(root, 'LICENSE'), path.join(stage, 'LICENSE'));
    await fs.copyFile(path.join(root, 'resources/fonts/Inter-OFL.txt'), path.join(stage, 'Inter-OFL.txt'));
    await fs.writeFile(path.join(stage, 'refract-native-installed'), `${manifest.version}\n`);
    await editWindowsResources({
      file: executable,
      fileVersion: manifest.version,
      productVersion: manifest.version.split('-')[0],
      iconPath,
      versionStrings: {
        FileDescription: product,
        ProductName: product,
        InternalName: product,
        OriginalFilename: executableName,
      },
    });

    const helper = path.resolve(dependencyHelper);
    await fs.access(helper);
    process.env.REFRACT_NSIS_DEPENDENCY_HELPER = helper;
    // The prepackaged path gives the existing NSIS builder a Rust payload.
    // It keeps the installer ID, shortcuts, update arguments and per-user paths.
    await build({
      projectDir: root,
      targets: createTargets([Platform.WINDOWS], 'nsis', 'x64'),
      prepackaged: stage,
      publish: 'never',
      config: {
        directories: { output: outputDir },
        win: { signExecutable: false },
        nsis: {
          include: 'scripts/native-windows/installer.nsh',
          artifactName: releaseAssets ? '${productName}-Setup-${version}.${ext}' : '${productName}-Native-Setup-${version}.${ext}',
          packElevateHelper: false,
          ...(releaseAssets && github && typeof github === 'object' ? { publish: { ...github, channel } } : {}),
        },
      },
    });
    if (releaseAssets) await verifyAndMarkReleaseAssets(outputDir, product, version, channel);
  } finally {
    const resolved = await fs.realpath(stage);
    if (path.dirname(resolved) !== temp || !path.basename(resolved).startsWith('refract-native-payload-')) {
      throw new Error(`Unexpected package staging path ${resolved}`);
    }
    await fs.rm(resolved, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
}

async function verifyAndMarkReleaseAssets(output: string, product: string, version: string, channel: 'beta' | 'latest'): Promise<void> {
  const installer = `${product}-Setup-${version}.exe`;
  const channelFile = `${channel}.yml`;
  const checksum = createHash('sha512');
  const nativeChecksum = createHash('sha256');
  let size = 0;
  for await (const bytes of createReadStream(path.join(output, installer))) {
    size += bytes.length;
    checksum.update(bytes);
    nativeChecksum.update(bytes);
  }
  const sha512 = checksum.digest('base64');
  const metadata = parseUpdateInfo(await fs.readFile(path.join(output, channelFile), 'utf8'),
    channelFile, new URL(`https://example.invalid/${channelFile}`));
  const [file] = metadata.files ?? [];
  const blockmap = await fs.stat(path.join(output, `${installer}.blockmap`));
  if (metadata.version !== version || metadata.path !== installer || metadata.sha512 !== sha512
    || metadata.files?.length !== 1 || file?.url !== installer || file?.sha512 !== sha512 || file?.size !== size
    || size === 0 || !blockmap.isFile() || blockmap.size === 0) {
    throw new Error(`Windows update assets do not agree with ${channelFile}.`);
  }

  // Native uses this checksum together with GitHub's SHA-256 metadata to
  // recognize a Rust installer under the same asset name Electron used.
  const sha256 = nativeChecksum.digest('hex');
  await fs.writeFile(path.join(output, `${product}-Native-Setup-${version}.${sha256}.sha256`), `${sha256}  ${installer}\n`);
}

packageWindows(process.argv.slice(2)).catch((error: unknown) => {
  console.error(error);
  process.exitCode = 1;
});
