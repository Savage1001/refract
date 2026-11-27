import type { Configuration } from 'electron-builder';

const fs = require('node:fs/promises') as typeof import('node:fs/promises');
const os = require('node:os') as typeof import('node:os');
const path = require('node:path') as typeof import('node:path');
const { build, createTargets, Platform } = require('electron-builder') as typeof import('electron-builder');
const { editWindowsResources } = require('app-builder-lib/out/util/resEdit') as typeof import('app-builder-lib/out/util/resEdit');

async function packageWindows([app, dependencyHelper, output]: string[]): Promise<void> {
  if (process.platform !== 'win32' || process.arch !== 'x64' || !app || !dependencyHelper || !output) {
    throw new Error('On x64 Windows, pass the native application, dependency helper, and installer output directory.');
  }

  const root = path.resolve(__dirname, '../..');
  const manifest = JSON.parse(await fs.readFile(path.join(root, 'package.json'), 'utf8')) as {
    version: string;
    build: Configuration;
  };
  const product = manifest.build.productName;
  const executableName = `${product}.exe`;
  const iconPath = path.join(root, 'resources/logo.ico');
  const temp = await fs.realpath(os.tmpdir());
  const stage = await fs.mkdtemp(path.join(temp, 'refract-native-payload-'));
  try {
    const executable = path.join(stage, executableName);
    await fs.copyFile(path.resolve(app), executable);
    await fs.copyFile(path.join(root, 'LICENSE'), path.join(stage, 'LICENSE'));
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
        directories: { output: path.resolve(output) },
        win: { signExecutable: false },
        nsis: {
          include: 'scripts/native-windows/installer.nsh',
          artifactName: '${productName}-Native-Setup-${version}.${ext}',
          packElevateHelper: false,
        },
      },
    });
  } finally {
    const resolved = await fs.realpath(stage);
    if (path.dirname(resolved) !== temp || !path.basename(resolved).startsWith('refract-native-payload-')) {
      throw new Error(`Unexpected package staging path ${resolved}`);
    }
    await fs.rm(resolved, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
}

packageWindows(process.argv.slice(2)).catch((error: unknown) => {
  console.error(error);
  process.exitCode = 1;
});
