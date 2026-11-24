import path = require('node:path');
import fs = require('node:fs');

const latexEngines = ['pdflatex', 'xelatex', 'lualatex'] as const;
type LatexEngine = typeof latexEngines[number];
function isLatexEngine(value: unknown): value is LatexEngine {
  return latexEngines.some(engine => engine === value);
}

function texExecutable(directory: string, tool: string): string {
  return path.join(directory, process.platform === 'win32' ? `${tool}.exe` : tool);
}

/** An explicit external toolchain takes priority over installer-managed TeX. */
function texBin(env: NodeJS.ProcessEnv = process.env): string {
  let directory = env.REFRACT_TEX_BIN;
  if (!directory && process.platform === 'win32' && env.LOCALAPPDATA) {
    const managed = path.join(env.LOCALAPPDATA, 'Refract', 'dependencies', 'TinyTeX', 'bin', 'windows');
    const complete = [...latexEngines, 'kpsewhich', 'synctex', 'bibtex', 'biber'].every(tool => {
      try { return fs.statSync(texExecutable(managed, tool)).isFile(); } catch { return false; }
    });
    if (complete) directory = managed;
  }
  if (!directory && process.platform === 'darwin' && fs.existsSync('/Library/TeX/texbin/pdflatex')) {
    directory = '/Library/TeX/texbin';
  }
  if (!directory || !path.isAbsolute(directory)) {
    if (process.platform === 'darwin') {
      throw new Error('Install MacTeX, or set REFRACT_TEX_BIN to the absolute bin directory of a separately installed TeX distribution, then restart Refract.');
    }
    throw new Error('Run the Refract installer with internet access to set up TeX, or set REFRACT_TEX_BIN to the absolute bin directory of a separately installed TeX distribution, then restart Refract.');
  }
  return path.normalize(directory);
}

export = { texBin, texExecutable, latexEngines, isLatexEngine };
