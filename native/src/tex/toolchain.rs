use std::{
    env, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LatexEngine {
    PdfLatex,
    XeLatex,
    LuaLatex,
}

impl LatexEngine {
    pub fn name(self) -> &'static str {
        match self {
            Self::PdfLatex => "pdflatex",
            Self::XeLatex => "xelatex",
            Self::LuaLatex => "lualatex",
        }
    }

    pub(super) fn parse(engine: &str) -> io::Result<Self> {
        match engine {
            "" | "pdflatex" => Ok(Self::PdfLatex),
            "xelatex" => Ok(Self::XeLatex),
            "lualatex" => Ok(Self::LuaLatex),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unsupported LaTeX engine in project settings",
            )),
        }
    }
}

pub struct TexToolchain {
    bin: PathBuf,
}

impl TexToolchain {
    pub fn discover() -> io::Result<Self> {
        if let Some(explicit) = env::var_os("REFRACT_TEX_BIN").filter(|path| !path.is_empty()) {
            return Self::from_bin(PathBuf::from(explicit));
        }
        platform_toolchain()
            .map(|bin| Self { bin })
            .ok_or_else(missing_toolchain)
    }

    pub fn from_bin(bin: PathBuf) -> io::Result<Self> {
        if !bin.is_absolute() {
            return Err(missing_toolchain());
        }
        Ok(Self { bin })
    }

    pub fn bin(&self) -> &Path {
        &self.bin
    }

    pub fn sync_executable(&self) -> PathBuf {
        self.executable("synctex")
    }

    pub(super) fn executable(&self, name: &str) -> PathBuf {
        tex_executable(&self.bin, name)
    }
}

fn tex_executable(bin: &Path, name: &str) -> PathBuf {
    if cfg!(windows) {
        bin.join(format!("{name}.exe"))
    } else {
        bin.join(name)
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn complete(bin: &Path) -> bool {
    [
        "pdflatex",
        "xelatex",
        "lualatex",
        "kpsewhich",
        "synctex",
        "bibtex",
        "biber",
    ]
    .iter()
    .all(|tool| tex_executable(bin, tool).is_file())
}

#[cfg(windows)]
fn platform_toolchain() -> Option<PathBuf> {
    let bin = PathBuf::from(env::var_os("LOCALAPPDATA")?)
        .join("Refract/dependencies/TinyTeX/bin/windows");
    (bin.is_absolute() && complete(&bin)).then_some(bin)
}

#[cfg(target_os = "macos")]
fn platform_toolchain() -> Option<PathBuf> {
    let managed = PathBuf::from("/Library/Refract/dependencies/TinyTeX/bin/universal-darwin");
    if complete(&managed) {
        return Some(managed);
    }
    let system = PathBuf::from("/Library/TeX/texbin");
    tex_executable(&system, "pdflatex")
        .is_file()
        .then_some(system)
}

#[cfg(target_os = "linux")]
fn platform_toolchain() -> Option<PathBuf> {
    use rustix::fs::{Access, access};

    env::split_paths(&env::var_os("PATH")?).find(|bin| {
        let engine = tex_executable(bin, "pdflatex");
        bin.is_absolute() && engine.is_file() && access(engine, Access::EXEC_OK).is_ok()
    })
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn platform_toolchain() -> Option<PathBuf> {
    None
}

fn missing_toolchain() -> io::Error {
    let message = if cfg!(target_os = "linux") {
        "Install TeX Live and SyncTeX so pdflatex is on PATH, or set REFRACT_TEX_BIN to the absolute bin directory of a separately installed TeX distribution, then restart Refract."
    } else {
        "Run the Refract installer with internet access to set up TeX, or set REFRACT_TEX_BIN to the absolute bin directory of an existing TeX distribution, then restart Refract."
    };
    io::Error::new(io::ErrorKind::NotFound, message)
}
