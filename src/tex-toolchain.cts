import path = require('node:path');
import fs = require('node:fs');

const latexEngines = ['pdflatex', 'xelatex', 'lualatex'] as const;
const requiredTexTools = [...latexEngines, 'kpsewhich', 'synctex', 'bibtex', 'biber'];
const macDependencies = '/Library/Application Support/Refract/dependencies';
const macTexSubdirectory = 'bin/universal-darwin';
type LatexEngine = typeof latexEngines[number];
function isLatexEngine(value: unknown): value is LatexEngine {
  return latexEngines.some(engine => engine === value);
}

function texExecutable(directory: string, tool: string): string {
  return path.join(directory, process.platform === 'win32' ? `${tool}.exe` : tool);
}

function hasCompleteToolchain(directory: string): boolean {
  return requiredTexTools.every(tool => {
    try { return fs.statSync(texExecutable(directory, tool)).isFile(); } catch { return false; }
  });
}

/** An explicit external toolchain takes priority over installer-managed TeX. */
function texBin(env: NodeJS.ProcessEnv = process.env): string {
  let directory = env.REFRACT_TEX_BIN;
  if (!directory && process.platform === 'win32' && env.LOCALAPPDATA) {
    const managed = path.join(env.LOCALAPPDATA, 'Refract', 'dependencies', 'TinyTeX', 'bin', 'windows');
    if (hasCompleteToolchain(managed)) directory = managed;
  }
  if (!directory && process.platform === 'darwin') {
    const managed = path.join(macDependencies, 'TinyTeX', macTexSubdirectory);
    if (hasCompleteToolchain(managed)) directory = managed;
    else if (fs.existsSync('/Library/TeX/texbin/pdflatex')) directory = '/Library/TeX/texbin';
  }
  if (!directory && process.platform === 'linux') {
    directory = env.PATH?.split(path.delimiter).find(entry => {
      if (!path.isAbsolute(entry)) return false;
      try {
        const engine = texExecutable(entry, 'pdflatex');
        fs.accessSync(engine, fs.constants.X_OK);
        return fs.statSync(engine).isFile();
      } catch {
        return false;
      }
    });
  }
  if (!directory || !path.isAbsolute(directory)) {
    if (process.platform === 'darwin') {
      throw new Error('Run the Refract installer with internet access to set up TeX, or set REFRACT_TEX_BIN to the absolute bin directory of an existing TeX distribution, then restart Refract.');
    }
    if (process.platform === 'linux') {
      throw new Error('Install TeX Live and SyncTeX so pdflatex is on PATH, or set REFRACT_TEX_BIN to the absolute bin directory of a separately installed TeX distribution, then restart Refract.');
    }
    throw new Error('Run the Refract installer with internet access to set up TeX, or set REFRACT_TEX_BIN to the absolute bin directory of a separately installed TeX distribution, then restart Refract.');
  }
  return path.normalize(directory);
}

export = { texBin, texExecutable, latexEngines, isLatexEngine, requiredTexTools, macDependencies, macTexSubdirectory };
