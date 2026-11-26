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
        build(&self.dir, &settings, &self.toolchain, cancel, runner)
    }
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
        project.dir.join("sections/body.tex"),
        "A Refract compiler fixture.\n",
    )
    .unwrap();
    fs::write(
        project.dir.join("main.tex"),
        br"\documentclass{article}
\begin{document}
\input{sections/body}
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
    )
    .unwrap();
    let Some(pdf) = result.pdf else {
        panic!("Compilation failed. {}", result.log);
    };
    assert_eq!(result.exit_code, Some(0));
    assert!(fs::read(pdf).unwrap().starts_with(b"%PDF-"));
    assert!(project.output.join("main.synctex.gz").is_file());
}
