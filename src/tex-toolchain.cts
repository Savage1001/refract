import path = require('node:path');
import fs = require('node:fs');

const latexEngines = ['pdflatex', 'xelatex', 'lualatex'] as const;
type LatexEngine = typeof latexEngines[number];
function isLatexEngine(value: unknown): value is LatexEngine {
  return latexEngines.some(engine => engine === value);
}

/** An explicit external toolchain takes priority over installer-managed TeX. */
function texBin(env: NodeJS.ProcessEnv = process.env): string {
  const managed = env.LOCALAPPDATA && path.join(env.LOCALAPPDATA, 'Refract', 'dependencies', 'TinyTeX', 'bin', 'windows');
  const complete = managed && [...latexEngines, 'kpsewhich', 'synctex', 'bibtex', 'biber'].every(tool => {
    try { return fs.statSync(path.join(managed, `${tool}.exe`)).isFile(); } catch { return false; }
  });
  const directory = env.REFRACT_TEX_BIN || (complete ? managed : undefined);
  if (!directory || !path.isAbsolute(directory)) {
    throw new Error('Run the Refract installer with internet access to set up TeX, or set REFRACT_TEX_BIN to the absolute bin directory of a separately installed TeX distribution, then restart Refract.');
  }
  return path.normalize(directory);
}

export = { texBin, latexEngines, isLatexEngine };
