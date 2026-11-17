param([string]$TexBin = $env:REFRACT_TEX_BIN)
$ErrorActionPreference = 'Stop'
# Validate only: never download, copy, modify, or run a TeX distribution.
if (-not $TexBin -or -not [IO.Path]::IsPathRooted($TexBin)) {
  throw 'Supply -TexBin with the absolute bin directory of a separately installed TeX Live or TinyTeX distribution.'
}
foreach ($name in @('pdflatex', 'xelatex', 'lualatex', 'kpsewhich', 'synctex', 'bibtex', 'biber')) {
  if (-not (Test-Path -LiteralPath (Join-Path $TexBin "$name.exe") -PathType Leaf)) {
    throw "Missing $name.exe in the supplied TeX bin directory. Install/configure it separately; Refract does not include TeX."
  }
}
Write-Output 'TeX tool paths exist (executables were not run).'
Write-Output 'Set REFRACT_TEX_BIN to this directory in the environment used to launch Refract, then restart Refract.'
