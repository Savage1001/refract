import path = require('node:path');

/** TeX is supplied by the user, never by the Refract installer. */
function texBin(env: NodeJS.ProcessEnv = process.env): string {
  const directory = env.REFRACT_TEX_BIN;
  if (!directory || !path.isAbsolute(directory)) {
    throw new Error('Set REFRACT_TEX_BIN to the absolute bin directory of your separately installed TeX Live or TinyTeX distribution, then restart Refract. Refract does not include TeX.');
  }
  return path.normalize(directory);
}

export = { texBin };
