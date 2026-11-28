use super::*;
use crate::create_project;
use tempfile::TempDir;

struct Fixture {
    _profile: TempDir,
    projects: PathBuf,
    project: Project,
    dir: PathBuf,
    output: PathBuf,
    toolchain: TexToolchain,
}

impl Fixture {
    fn new() -> Self {
        let profile = tempfile::tempdir().unwrap();
        let projects = profile.path().join("projects");
        let project = create_project(&projects, "Paper").unwrap();
        let dir = projects.join(&project.id).canonicalize().unwrap();
        let output = dir.join(".refract-build");
        let toolchain = TexToolchain::from_bin(profile.path().join("tex/bin")).unwrap();
        Self {
            _profile: profile,
            projects,
            project,
            dir,
            output,
            toolchain,
        }
    }

    fn build(
        &self,
        cancel: &AtomicBool,
        runner: impl FnMut(Command, &AtomicBool) -> io::Result<ProcessOutput>,
    ) -> io::Result<BuildResult> {
        let settings = build_settings(&self.projects, &self.project)?;
        build(
            &self.dir,
            &settings,
            &self.toolchain,
            cancel,
            false,
            runner,
            |_| {},
        )
    }
}

#[test]
fn keeps_disk_and_memory_previews_when_the_build_replaces_its_pdf() {
    let fixture = Fixture::new();
    fs::create_dir(&fixture.output).unwrap();
    let pdf = fixture.output.join("main.pdf");
    let original = b"%PDF-1.4\nPrevious document\n";
    let cancel = AtomicBool::new(false);

    for memory_limit in [0, original.len() as u64] {
        fs::write(&pdf, original).unwrap();
        let snapshot = snapshot_pdf(build_file(&pdf).unwrap(), memory_limit, &cancel).unwrap();
        fs::remove_file(&pdf).unwrap();
        fs::write(&pdf, b"%PDF-1.4\nReplacement\n").unwrap();

        match snapshot {
            RetainedPdf::Bytes(bytes) => {
                assert_ne!(memory_limit, 0);
                assert_eq!(bytes.as_slice(), original);
            }
            RetainedPdf::File(file) => {
                assert_eq!(memory_limit, 0);
                #[cfg(windows)]
                {
                    let path = file.path().to_owned();
                    assert_eq!(fs::read(&path).unwrap(), original);
                    assert!(fs::write(&path, b"overwrite").is_err());
                    assert!(fs::remove_file(&path).is_err());
                    drop(file);
                    assert!(!path.exists());
                }
                #[cfg(all(
                    feature = "portable-pdf",
                    any(target_os = "linux", target_os = "macos")
                ))]
                assert_eq!(file.map().unwrap().as_ref(), original);
                #[cfg(not(any(
                    windows,
                    all(
                        feature = "portable-pdf",
                        any(target_os = "linux", target_os = "macos")
                    )
                )))]
                {
                    use std::io::{Seek, SeekFrom};
                    let mut file = &file.file;
                    file.seek(SeekFrom::Start(0)).unwrap();
                    let mut bytes = Vec::new();
                    file.read_to_end(&mut bytes).unwrap();
                    assert_eq!(bytes, original);
                }
            }
        }
    }
}

#[test]
fn captures_the_pdf_while_the_build_lock_is_held() {
    let fixture = Fixture::new();
    let cancel = AtomicBool::new(false);
    let settings = build_settings(&fixture.projects, &fixture.project).unwrap();
    let mut snapshot = None;
    build(
        &fixture.dir,
        &settings,
        &fixture.toolchain,
        &cancel,
        false,
        |_, _| {
            fs::write(fixture.output.join("main.pdf"), b"%PDF-fixture")?;
            Ok(success("rendered"))
        },
        |pdf| {
            let error = lock_build_directory(&fixture.output).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
            snapshot = Some(retain_pdf_at(pdf, &cancel).unwrap());
        },
    )
    .unwrap();
    assert!(matches!(snapshot, Some(RetainedPdf::Bytes(_))));
}

#[test]
fn reuses_only_verified_outputs_and_honors_forced_builds() {
    let fixture = Fixture::new();
    let dependency = fixture._profile.path().join("external.sty");
    fs::write(&dependency, "before").unwrap();
    fs::create_dir_all(fixture.toolchain.bin()).unwrap();
    fs::write(fixture.toolchain.executable("pdflatex"), "engine").unwrap();
    let settings = build_settings(&fixture.projects, &fixture.project).unwrap();
    let cancel = AtomicBool::new(false);
    let compile = |force, may_run: bool| {
        build(
            &fixture.dir,
            &settings,
            &fixture.toolchain,
            &cancel,
            force,
            |_, _| {
                assert!(may_run, "the engine ran despite a current cached build");
                for (name, data) in [
                    ("main.pdf", "%PDF-fixture"),
                    ("main.log", "Build log"),
                    ("main.synctex.gz", "Sync data"),
                    ("main.aux", "\\relax\n"),
                ] {
                    fs::write(fixture.output.join(name), data)?;
                }
                fs::write(
                    fixture.output.join("main.fls"),
                    format!(
                        "INPUT ./main.tex\nINPUT {}\nOUTPUT {}\n",
                        dependency.display(),
                        tool_path(&fixture.output).join("main.aux").display(),
                    ),
                )?;
                Ok(success("rendered"))
            },
            |path| assert!(lock_build_directory(path.parent().unwrap()).is_err()),
        )
        .unwrap()
        .cached
    };
    assert!(!compile(false, true));
    assert!(compile(false, false));
    assert!(!compile(true, true));
    fs::write(&dependency, "change").unwrap();
    assert!(!compile(false, true));
    fs::write(fixture.dir.join("extra.tex"), "a newly visible file").unwrap();
    assert!(!compile(false, true));
    fs::remove_file(fixture.output.join("main.synctex.gz")).unwrap();
    assert!(!compile(false, true));
    assert!(compile(false, false));
}

#[test]
fn preview_rejects_invalid_and_linked_build_artifacts_and_honors_cancellation() {
    let fixture = Fixture::new();
    fs::create_dir(&fixture.output).unwrap();
    let pdf = fixture.output.join("main.pdf");
    let cancel = AtomicBool::new(false);
    let read = || retain_compiled_pdf(&fixture.projects, &fixture.project, &cancel);
    fs::write(&pdf, b"incomplete build").unwrap();
    assert_eq!(read().unwrap_err().kind(), io::ErrorKind::InvalidData);
    fs::write(&pdf, b"%PDF-1.4\n").unwrap();
    cancel.store(true, Ordering::Relaxed);
    assert_eq!(read().unwrap_err().kind(), io::ErrorKind::Interrupted);
    cancel.store(false, Ordering::Relaxed);
    fs::hard_link(&pdf, fixture.dir.join("linked.pdf")).unwrap();
    assert_eq!(read().unwrap_err().kind(), io::ErrorKind::InvalidData);
}

fn tool_name(command: &Command) -> String {
    Path::new(command.get_program())
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn success(log: &str) -> ProcessOutput {
    ProcessOutput {
        code: Some(0),
        log: log.to_owned(),
    }
}

#[test]
fn reads_only_the_compiled_pdf_for_the_library_thumbnail() {
    let fixture = Fixture::new();
    let read = || compiled_pdf_for_thumbnail(&fixture.projects, &fixture.project);
    assert!(read().unwrap().is_none());
    fs::create_dir(&fixture.output).unwrap();
    let pdf = fixture.output.join("main.pdf");
    fs::write(&pdf, b"incomplete build").unwrap();
    assert!(read().is_err());
    let bytes = b"%PDF-1.4\n";
    fs::write(&pdf, bytes).unwrap();
    assert_eq!(read().unwrap().as_deref(), Some(bytes.as_slice()));
    fs::hard_link(&pdf, fixture.dir.join("linked.pdf")).unwrap();
    assert!(read().is_err());
}

#[test]
fn loads_engine_root_and_draft_mode_from_current_metadata() {
    let project = Fixture::new();
    let metadata = project.dir.join(".refract.json");
    let mut properties: Value = serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
    properties["engine"] = "xelatex".into();
    properties["rootFile"] = r"sections\report%2026.tex".into();
    properties["draftMode"] = true.into();
    fs::write(&metadata, properties.to_string()).unwrap();
    fs::create_dir(project.dir.join("sections")).unwrap();
    fs::write(project.dir.join("sections/report%2026.tex"), "Report").unwrap();
    let build = project
        .build(&AtomicBool::new(false), |command, _| {
            assert_eq!(tool_name(&command), "xelatex");
            assert_eq!(
                command.get_args().last().unwrap(),
                "./.refract-build/refract-internal/draft.tex"
            );
            assert!(command.get_args().any(|arg| arg == "-no-shell-escape"));
            assert!(command.get_args().any(|arg| arg == "-synctex=1"));
            let wrapper =
                fs::read_to_string(project.output.join("refract-internal/draft.tex")).unwrap();
            assert!(wrapper.contains(r"\readline\refractrootstream"));
            let root =
                fs::read_to_string(project.output.join("refract-internal/draft-root.txt")).unwrap();
            assert!(root.ends_with("/sections/report%2026.tex\n"));
            fs::write(project.output.join("main.pdf"), b"%PDF-fixture").unwrap();
            Ok(success("rendered"))
        })
        .unwrap();
    assert_eq!(
        build.pdf.as_deref(),
        Some(project.output.join("main.pdf").as_path())
    );
    properties["engine"] = "unsupported".into();
    fs::write(&metadata, properties.to_string()).unwrap();
    assert!(build_settings(&project.projects, &project.project).is_err());
}

#[test]
fn skips_the_second_pass_only_when_the_recorder_and_aux_are_complete() {
    for (extra, passes) in [("", 1), ("OUTPUT extra.toc\n", 2), ("INPUT main.aux\n", 2)] {
        let project = Fixture::new();
        let mut actual = 0;
        let result = project
            .build(&AtomicBool::new(false), |command, _| {
                assert_eq!(tool_name(&command), "pdflatex");
                assert_eq!(command.get_args().last().unwrap(), "./main.tex");
                assert_eq!(command.get_current_dir(), Some(tool_path(&project.dir)));
                actual += 1;
                fs::write(
                    project.output.join("main.aux"),
                    "\\relax\r\n\\gdef \\@abspage@last{1}\r\n",
                )
                .unwrap();
                fs::write(
                    project.output.join("main.fls"),
                    format!(
                        "OUTPUT .refract-build/main.aux\nOUTPUT .refract-build/main.pdf\n{extra}"
                    ),
                )
                .unwrap();
                fs::write(project.output.join("main.pdf"), b"%PDF-fixture").unwrap();
                Ok(success("No file main.aux.\n"))
            })
            .unwrap();
        assert!(result.pdf.is_some());
        assert_eq!(actual, passes);
    }
}

#[test]
fn runs_bibliography_tools_and_the_additional_latex_passes() {
    for bibliography in ["bibtex", "biber"] {
        let project = Fixture::new();
        let mut runs = Vec::new();
        let result = project
            .build(&AtomicBool::new(false), |command, _| {
                let tool = tool_name(&command);
                if runs.is_empty() {
                    if bibliography == "bibtex" {
                        fs::write(project.output.join("main.aux"), br"\bibdata{references}")
                            .unwrap();
                    } else {
                        fs::write(project.output.join("main.bcf"), b"biber fixture").unwrap();
                    }
                }
                if tool == bibliography {
                    assert_eq!(command.get_args().last().unwrap(), "main");
                    if tool == "bibtex" {
                        assert_eq!(command.get_current_dir(), Some(tool_path(&project.output)));
                    } else {
                        assert!(command.get_args().any(|arg| arg == "--input-directory"));
                        assert!(
                            command
                                .get_args()
                                .any(|arg| arg == tool_path(&project.output).as_os_str())
                        );
                    }
                }
                runs.push(tool);
                fs::write(project.output.join("main.pdf"), b"%PDF-fixture").unwrap();
                Ok(success("rendered"))
            })
            .unwrap();
        assert_eq!(runs, ["pdflatex", bibliography, "pdflatex", "pdflatex"]);
        assert!(result.pdf.is_some());
    }
}

#[test]
fn removes_pdf_and_synctex_output_from_failed_or_cancelled_builds() {
    for cancelled in [false, true] {
        let project = Fixture::new();
        fs::create_dir(&project.output).unwrap();
        fs::write(project.output.join("main.pdf"), b"%PDF-old").unwrap();
        fs::write(project.output.join("main.synctex.gz"), b"old").unwrap();
        let result = project
            .build(&AtomicBool::new(false), |_, cancel| {
                assert!(!project.output.join("main.pdf").exists());
                assert!(!project.output.join("main.synctex.gz").exists());
                fs::write(project.output.join("main.pdf"), b"%PDF-incomplete").unwrap();
                fs::write(project.output.join("main.synctex.gz"), b"partial").unwrap();
                cancel.store(cancelled, Ordering::Relaxed);
                Ok(ProcessOutput {
                    code: if cancelled { None } else { Some(1) },
                    log: "Compilation stopped".into(),
                })
            })
            .unwrap();
        assert!(result.pdf.is_none());
        assert!(result.log.contains("Compilation stopped"));
        assert!(!project.output.join("main.pdf").exists());
        assert!(!project.output.join("main.synctex.gz").exists());
    }
}

#[test]
fn rejects_hard_linked_build_outputs_and_serializes_compilation() {
    let project = Fixture::new();
    fs::create_dir(&project.output).unwrap();
    let original = project.dir.join("main.tex");
    let pdf = project.output.join("main.pdf");
    fs::hard_link(&original, &pdf).unwrap();
    let before = fs::read(&original).unwrap();
    let cancel = AtomicBool::new(false);
    let result = project.build(&cancel, |_, _| {
        panic!("unsafe output directory reached the compiler")
    });
    assert_eq!(result.err().unwrap().kind(), io::ErrorKind::InvalidData);
    assert_eq!(fs::read(&original).unwrap(), before);
    assert!(pdf.exists());
    fs::remove_file(pdf).unwrap();

    let lock = lock_build_directory(&project.output).unwrap();
    let result = project.build(&cancel, |_, _| {
        panic!("another build still owns this output directory")
    });
    assert_eq!(result.err().unwrap().kind(), io::ErrorKind::WouldBlock);
    drop(lock);
}

#[test]
#[ignore = "requires TeX and launches a compiler, run by the Windows CI fixture"]
fn compiles_with_the_configured_toolchain() {
    let project = Fixture::new();
    fs::create_dir(project.dir.join("sections")).unwrap();
    fs::write(
        project.dir.join("sections/résumé.tex"),
        "A Refract compiler fixture.\n",
    )
    .unwrap();
    fs::write(
        project.dir.join("main.tex"),
        r"\documentclass{article}
\begin{document}
\input{sections/résumé}
\end{document}
",
    )
    .unwrap();
    let toolchain = TexToolchain::discover().unwrap();
    let result = compile_project(
        &project.projects,
        &project.project,
        &toolchain,
        &AtomicBool::new(false),
        false,
    )
    .unwrap();
    let Some(pdf) = result.pdf else {
        panic!("Compilation failed. {}", result.log);
    };
    assert_eq!(result.exit_code, Some(0));
    assert!(fs::read(pdf).unwrap().starts_with(b"%PDF-"));
    assert!(project.output.join("main.synctex.gz").is_file());
    let cached = compile_project(
        &project.projects,
        &project.project,
        &toolchain,
        &AtomicBool::new(false),
        false,
    )
    .unwrap();
    assert!(
        cached.cached,
        "unchanged TeX inputs should reuse the last build"
    );
    let position = synctex::forward(
        &project.projects,
        &project.project,
        &toolchain,
        "sections/résumé.tex",
        1,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(position.page, 1);
    let source = synctex::reverse(
        &project.projects,
        &project.project,
        &toolchain,
        position,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(source.file, "sections/résumé.tex");
    assert_eq!(source.line, 1);
}
