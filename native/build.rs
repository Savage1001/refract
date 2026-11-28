fn main() {
    #[cfg(feature = "latex-syntax")]
    {
        const GENERATED: &str = "latex-syntax/generated";
        println!("cargo:rerun-if-changed={GENERATED}/parser.c");
        println!("cargo:rerun-if-changed={GENERATED}/tree_sitter/parser.h");
        cc::Build::new()
            .include(GENERATED)
            .file(format!("{GENERATED}/parser.c"))
            .warnings(false)
            .compile("refract_latex");
    }
}
