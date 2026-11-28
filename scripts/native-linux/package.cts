import childProcess = require('node:child_process');
import fs = require('node:fs');
import path = require('node:path');
import zlib = require('node:zlib');

const { execFileSync } = childProcess;
const {
  chmodSync, closeSync, copyFileSync, mkdirSync, mkdtempSync, openSync,
  readFileSync, readSync, readdirSync, renameSync, rmSync, statSync, writeFileSync,
} = fs;
const { dirname, join, resolve } = path;
const { gzipSync } = zlib;

type Manifest = {
  name: string;
  version: string;
  homepage: string;
  license: string;
  build: {
    productName: string;
    linux: { category: string; executableName: string; icon: string; maintainer: string };
    deb: { depends: string[] };
    pacman: { depends: string[] };
  };
};

const root = process.cwd();
const description = 'LaTeX editor with a live PDF preview';
const archiveOwner = ['--uid', '0', '--gid', '0', '--uname', 'root', '--gname', 'root'];

function packageLinux(binary: string, destination: string, releaseAssets: boolean): void {
  verifyX64Elf(binary);
  const manifest: Manifest = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
  const { name, version, build } = manifest;
  if (!/^\d+\.\d+\.\d+(?:-pre-release-\d+)?$/.test(version)) {
    throw new Error('Expected a Refract release version in package.json');
  }
  const { category, executableName, icon } = build.linux;
  const installPath = `/opt/${build.productName}/${executableName}`;
  mkdirSync(destination, { recursive: true });
  const stage = mkdtempSync(join(destination, '.refract-linux-'));
  try {
    const payload = join(stage, 'payload');
    const program = join(payload, installPath);
    installFile(binary, program, 0o755);

    const applications = join(payload, 'usr/share/applications');
    mkdirSync(applications, { recursive: true });
    writeFileSync(join(applications, `${executableName}.desktop`), [
      '[Desktop Entry]',
      'Type=Application',
      `Name=${build.productName}`,
      `Exec=${installPath} %U`,
      `Icon=${executableName}`,
      `StartupWMClass=${executableName}`,
      'Terminal=false',
      `Categories=${category};`,
      '',
    ].join('\n'));
    const icons = join(payload, 'usr/share/icons/hicolor/256x256/apps');
    mkdirSync(icons, { recursive: true });
    execFileSync('convert', [
      join(root, icon), '-resize', '256x256', join(icons, `${executableName}.png`),
    ]);

    const docs = join(payload, 'usr/share/doc', name);
    installFile(join(root, 'LICENSE'), join(docs, 'copyright'));
    installFile(join(root, 'resources/fonts/Inter-OFL.txt'), join(docs, 'Inter-OFL.txt'));
    for (const notice of readdirSync(join(root, 'native/licenses'))) {
      installFile(join(root, 'native/licenses', notice), join(docs, notice));
    }

    // Keep the package and launcher names used by the Electron installers.
    // The TeX entries in the existing distribution manifest include SyncTeX.
    const artifactPrefix = `${build.productName}-${version}${releaseAssets ? '' : '-native'}`;
    const debName = `${artifactPrefix}-linux-amd64.deb`;
    const archName = `${artifactPrefix}-archlinux-x64.pkg.tar.zst`;
    buildDebian(payload, stage, manifest, installPath, debName);
    buildArch(payload, stage, manifest, installPath, archName);
    for (const file of [debName, archName]) {
      renameSync(join(stage, file), join(destination, file));
      process.stdout.write(`${join(destination, file)}\n`);
    }
  } finally {
    rmSync(stage, { recursive: true, force: true });
  }
}

function verifyX64Elf(binary: string): void {
  const header = Buffer.alloc(20);
  const handle = openSync(binary, 'r');
  try {
    if (readSync(handle, header) !== header.length
        || header.readUInt32BE(0) !== 0x7f454c46
        || header[4] !== 2 || header[5] !== 1
        || ![2, 3].includes(header.readUInt16LE(16))
        || header.readUInt16LE(18) !== 62) {
      throw new Error('Expected a Linux x86-64 Refract executable');
    }
  } finally {
    closeSync(handle);
  }
}

function installFile(source: string, target: string, mode = 0o644): void {
  mkdirSync(dirname(target), { recursive: true });
  copyFileSync(source, target);
  chmodSync(target, mode);
}

function texDependencies(dependencies: string[]): string[] {
  const extra = new Set(['biber', 'lmodern', 'otf-latin-modern']);
  return dependencies.filter((name) => name.startsWith('texlive-') || extra.has(name));
}

function buildDebian(payload: string, stage: string, manifest: Manifest, program: string, output: string): void {
  const { name, version, homepage, build } = manifest;
  const { executableName, maintainer } = build.linux;
  const metadata = join(payload, 'DEBIAN');
  const executable = join(payload, program);
  mkdirSync(metadata);
  try {
    // dpkg-shlibdeps uses the builder's symbol tables to find the actual
    // shared libraries and minimum supported library versions for this ELF.
    mkdirSync(join(stage, 'debian'));
    writeFileSync(join(stage, 'debian/control'), [
      `Source: ${name}`,
      `Maintainer: ${maintainer}`,
      '',
      `Package: ${name}`,
      'Architecture: amd64',
      `Description: ${description}`,
      '',
    ].join('\n'));
    const shared = execFileSync('dpkg-shlibdeps', ['-O', executable], {
      cwd: stage, encoding: 'utf8',
    }).match(/^shlibs:Depends=(.+)$/m)?.[1];
    if (!shared) throw new Error('Could not determine the native app shared library dependencies');
    // GPUI loads some libraries at runtime and uses a desktop portal for file pickers.
    const linkedNames = new Set(shared.split(',').map((entry) => entry.trim().split(' ')[0]));
    const loaded = ['libfontconfig1', 'libwayland-client0', 'libvulkan1'];
    const dependencies = [
      shared, ...loaded.filter((dependency) => !linkedNames.has(dependency)),
      'mesa-vulkan-drivers | vulkan-icd', 'fonts-liberation', 'xdg-utils', 'xdg-desktop-portal-gtk',
      ...texDependencies(build.deb.depends),
    ];
    writeFileSync(join(metadata, 'control'), [
      `Package: ${name}`,
      `Version: ${version.replace(/-/g, '~')}`,
      'Architecture: amd64',
      `Maintainer: ${maintainer}`,
      'Section: editors',
      'Priority: optional',
      `Installed-Size: ${Math.ceil(payloadBytes(payload) / 1024)}`,
      `Homepage: ${homepage}`,
      `Depends: ${dependencies.join(', ')}`,
      `Description: ${description}`,
      '',
    ].join('\n'));
    // Reuse the same alternatives registration as the previous packages.
    writeFileSync(join(metadata, 'postinst'), `#!/bin/sh
set -e
if [ "$1" = configure ]; then
  update-alternatives --install "/usr/bin/${executableName}" "${executableName}" "${program}" 100
fi
`, { mode: 0o755 });
    writeFileSync(join(metadata, 'prerm'), `#!/bin/sh
set -e
if [ "$1" = remove ]; then
  update-alternatives --remove "${executableName}" "${program}"
fi
`, { mode: 0o755 });
    execFileSync('dpkg-deb', ['--root-owner-group', '--build', payload, join(stage, output)], {
      stdio: 'inherit',
    });
  } finally {
    rmSync(metadata, { recursive: true, force: true });
  }
}

function buildArch(payload: string, stage: string, manifest: Manifest, program: string, output: string): void {
  const { name, version, homepage, license, build } = manifest;
  const dependencies = [
    'glibc', 'libgcc', 'libstdc++', 'fontconfig', 'libx11', 'libxcb',
    'libxkbcommon', 'libxkbcommon-x11', 'wayland', 'vulkan-icd-loader',
    'vulkan-driver', 'ttf-liberation', 'xdg-utils', 'xdg-desktop-portal-gtk', ...texDependencies(build.pacman.depends),
  ];
  const installedSize = payloadBytes(payload);
  writeFileSync(join(payload, '.PKGINFO'), [
    `pkgname = ${name}`,
    `pkgbase = ${name}`,
    'xdata = pkgtype=pkg',
    `pkgver = ${version.replace(/-/g, '_')}-1`,
    `pkgdesc = ${description}`,
    `url = ${homepage}`,
    `builddate = ${Math.floor(Date.now() / 1000)}`,
    `packager = ${build.linux.maintainer}`,
    `size = ${installedSize}`,
    'arch = x86_64',
    `license = ${license}`,
    ...dependencies.map((dependency) => `depend = ${dependency}`),
    '',
  ].join('\n'));
  // Previously installed Arch packages may leave this link unowned. Keep
  // using an install hook so pacman can upgrade without a file conflict.
  const launcher = `/usr/bin/${build.linux.executableName}`;
  writeFileSync(join(payload, '.INSTALL'), `post_install() {
  if [ ! -e "${launcher}" ] && [ ! -L "${launcher}" ]; then
    ln -s "${program}" "${launcher}"
  fi
}

post_upgrade() {
  post_install
}

post_remove() {
  if [ "$(readlink "${launcher}")" = "${program}" ]; then
    rm "${launcher}"
  fi
}
`);
  const entries = ['.PKGINFO', '.INSTALL', 'opt', 'usr'];
  const contents = execFileSync('bsdtar', [
    '-cf', '-', '--format=mtree', '--options=mtree:sha256', ...archiveOwner, ...entries,
  ], { cwd: payload });
  writeFileSync(join(payload, '.MTREE'), gzipSync(contents));
  execFileSync('bsdtar', [
    '-cf', join(stage, output), '--zstd', '--format=pax', ...archiveOwner,
    '.PKGINFO', '.INSTALL', '.MTREE', 'opt', 'usr',
  ], { cwd: payload, stdio: 'inherit' });
}

function payloadBytes(directory: string): number {
  return readdirSync(directory, { withFileTypes: true }).reduce((size, entry) => {
    const entryPath = join(directory, entry.name);
    return size + (entry.isDirectory() ? payloadBytes(entryPath) : statSync(entryPath).size);
  }, 0);
}

const [binary, destination, mode, ...extra] = process.argv.slice(2);
const releaseAssets = mode === '--release-assets';
if (process.platform !== 'linux' || !binary || !destination || (mode !== undefined && !releaseAssets) || extra.length > 0) {
  throw new Error('On Linux, pass a built refract-native executable, an output directory, and optionally --release-assets');
}
packageLinux(resolve(binary), resolve(destination), releaseAssets);
