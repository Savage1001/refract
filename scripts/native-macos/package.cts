import childProcess = require('node:child_process');
import fs = require('node:fs');
import path = require('node:path');

const repository = path.resolve(__dirname, '../..');
const binaries = path.join(repository, 'native', 'target', 'release');
const distributions = path.join(repository, 'dist');
const output = path.join(distributions, 'native-macos');
const helper = 'refract-install-dependencies';

interface AppMetadata {
  version: string;
  build: {
    appId: string;
    productName: string;
  };
}

function run(program: string, ...args: string[]): string {
  return childProcess.execFileSync(program, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });
}

function writePlist(file: string, body: string): void {
  fs.writeFileSync(file, `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
${body}
</plist>
`);
  run('/usr/bin/plutil', '-lint', file);
}

function addIcon(stage: string, resources: string): void {
  const iconset = path.join(stage, 'Refract.iconset');
  const source = path.join(repository, 'resources', 'logo.png');
  fs.mkdirSync(iconset);
  for (const size of [16, 32, 128, 256, 512]) {
    run('/usr/bin/sips', '-z', String(size), String(size), source, '--out',
      path.join(iconset, `icon_${size}x${size}.png`));
    const doubled = String(size * 2);
    run('/usr/bin/sips', '-z', doubled, doubled, source, '--out',
      path.join(iconset, `icon_${size}x${size}@2x.png`));
  }
  run('/usr/bin/iconutil', '-c', 'icns', iconset, '-o', path.join(resources, 'Refract.icns'));
}

function minimumMacOS(binary: string, architecture: string): string {
  run('/usr/bin/lipo', binary, '-verify_arch', architecture);
  const loadCommands = run('/usr/bin/xcrun', 'vtool', '-arch', architecture, '-show-build', binary);
  const minimum = loadCommands.match(/^\s*(?:minos|version)\s+(\d+\.\d+(?:\.\d+)?)\s*$/m)?.[1];
  if (!minimum) throw new Error(`Couldn't read the macOS deployment target for ${binary}`);
  return minimum;
}

function packageReleaseImage(stage: string, pkg: string, stem: string): void {
  const source = path.join(stage, 'image');
  fs.mkdirSync(source);
  fs.copyFileSync(pkg, path.join(source, `${stem}.pkg`));
  const image = path.join(stage, `${stem}.dmg`);
  run('/usr/bin/hdiutil', 'create', '-volname', 'Refract Installer', '-srcfolder', source, '-format', 'UDZO', image);
  run('/usr/bin/hdiutil', 'verify', image);
  fs.renameSync(image, path.join(distributions, `${stem}.dmg`));
}

function packageApp(stage: string, metadata: AppMetadata, archiveArch: string, machArch: string, releaseAssets: boolean): void {
  const { appId, productName } = metadata.build;
  const [shortVersion, prerelease] = metadata.version.split('-pre-release-');
  const bundleVersion = prerelease ? `${shortVersion}b${prerelease}` : shortVersion;
  const root = path.join(stage, 'application');
  const bundle = path.join(root, `${productName}.app`);
  const contents = path.join(bundle, 'Contents');
  const executables = path.join(contents, 'MacOS');
  const resources = path.join(contents, 'Resources');
  fs.mkdirSync(executables, { recursive: true });
  fs.mkdirSync(resources);

  const appBinary = path.join(binaries, 'refract-native');
  const installerBinary = path.join(binaries, helper);
  const minimum = [appBinary, installerBinary]
    .map(binary => minimumMacOS(binary, machArch))
    .sort((left, right) => left.localeCompare(right, 'en', { numeric: true }))[1];

  fs.copyFileSync(appBinary, path.join(executables, productName));
  fs.copyFileSync(installerBinary, path.join(executables, helper));
  fs.chmodSync(path.join(executables, productName), 0o755);
  fs.chmodSync(path.join(executables, helper), 0o755);
  fs.copyFileSync(path.join(repository, 'LICENSE'), path.join(resources, 'LICENSE'));
  fs.cpSync(path.join(repository, 'native', 'licenses'), path.join(resources, 'licenses'), { recursive: true });
  fs.copyFileSync(path.join(repository, 'resources', 'fonts', 'Inter-OFL.txt'),
    path.join(resources, 'licenses', 'Inter-OFL.txt'));
  addIcon(stage, resources);
  writePlist(path.join(contents, 'Info.plist'), `<dict>
  <key>CFBundleIdentifier</key><string>${appId}</string>
  <key>CFBundleName</key><string>${productName}</string>
  <key>CFBundleDisplayName</key><string>${productName}</string>
  <key>CFBundleExecutable</key><string>${productName}</string>
  <key>CFBundleIconFile</key><string>Refract.icns</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${shortVersion}</string>
  <key>CFBundleVersion</key><string>${bundleVersion}</string>
  <key>LSMinimumSystemVersion</key><string>${minimum}</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>`);
  run('/usr/bin/codesign', '--force', '--sign', '-', '--timestamp=none', path.join(executables, helper));
  run('/usr/bin/codesign', '--force', '--sign', '-', '--timestamp=none', bundle);
  run('/usr/bin/codesign', '--verify', '--deep', '--strict', bundle);

  const component = path.join(stage, 'Component.plist');
  writePlist(component, `<array>
  <dict>
    <key>RootRelativeBundlePath</key><string>${productName}.app</string>
    <key>BundleIsRelocatable</key><false/>
    <key>BundleHasStrictIdentifier</key><true/>
    <key>BundleIsVersionChecked</key><true/>
    <key>BundleOverwriteAction</key><string>upgrade</string>
  </dict>
</array>`);
  const scripts = path.join(stage, 'scripts');
  fs.mkdirSync(scripts);
  const postinstall = fs.readFileSync(path.join(repository, 'scripts', 'native-macos', 'postinstall'), 'utf8');
  fs.writeFileSync(path.join(scripts, 'postinstall'), postinstall.replace(/\r\n/g, '\n'), { mode: 0o755 });

  const componentName = 'Refract-component.pkg';
  run('/usr/bin/pkgbuild', '--root', root, '--identifier', appId, '--component-plist', component,
    '--install-location', '/Applications', '--version', metadata.version,
    '--scripts', scripts, path.join(stage, componentName));
  const distribution = path.join(stage, 'Distribution.xml');
  fs.writeFileSync(distribution, `<?xml version="1.0" encoding="UTF-8"?>
<installer-gui-script minSpecVersion="2">
  <title>${productName}</title>
  <options customize="never" require-scripts="false" rootVolumeOnly="true" hostArchitectures="${machArch}"/>
  <domains enable_anywhere="false" enable_currentUserHome="false" enable_localSystem="true"/>
  <volume-check script="true">
    <allowed-os-versions><os-version min="${minimum}"/></allowed-os-versions>
  </volume-check>
  <choices-outline><line choice="refract"/></choices-outline>
  <choice id="refract" visible="false"><pkg-ref id="${appId}"/></choice>
  <pkg-ref id="${appId}" version="${metadata.version}">${componentName}</pkg-ref>
  <pkg-ref id="${appId}"><must-close><app id="${appId}"/></must-close></pkg-ref>
</installer-gui-script>
`);
  const artifact = `${productName}-native-${metadata.version}-mac-${archiveArch}.pkg`;
  run('/usr/bin/productbuild', '--distribution', distribution, '--package-path', stage, path.join(stage, artifact));
  const pkg = path.join(output, artifact);
  fs.renameSync(path.join(stage, artifact), pkg);
  if (releaseAssets) packageReleaseImage(stage, pkg, `${productName}-${metadata.version}-mac-${archiveArch}`);
}

function main(): void {
  const args = process.argv.slice(2);
  const releaseAssets = args.length === 1 && args[0] === '--release-assets';
  if (args.length && !releaseAssets) throw new Error('Expected no arguments or --release-assets');
  if (process.platform !== 'darwin' || !['arm64', 'x64'].includes(process.arch)) {
    throw new Error('Build the macOS package on an Apple Silicon or Intel Mac');
  }
  const metadata = JSON.parse(fs.readFileSync(path.join(repository, 'package.json'), 'utf8')) as AppMetadata;
  if (!/^\d+\.\d+\.\d+(?:-pre-release-[1-9]\d*)?$/.test(metadata.version) ||
      metadata.build.productName !== 'Refract' || !/^[\w.-]+$/.test(metadata.build.appId)) {
    throw new Error('Invalid Refract package metadata');
  }
  fs.mkdirSync(output, { recursive: true });
  const stage = fs.mkdtempSync(path.join(output, 'stage-'));
  try {
    packageApp(stage, metadata, process.arch, process.arch === 'arm64' ? 'arm64' : 'x86_64', releaseAssets);
  } finally {
    fs.rmSync(stage, { recursive: true, force: true });
  }
}

main();
