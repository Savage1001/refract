use refract_native::managed_tex::{setup, write_setup_error};

fn main() -> std::process::ExitCode {
    match setup() {
        Ok(path) => {
            println!("TeX and SyncTeX ready at {}", path.display());
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("TeX and SyncTeX setup failed. {error}");
            if let Ok(log) = write_setup_error(&error.to_string()) {
                eprintln!("See {}", log.display());
            }
            std::process::ExitCode::FAILURE
        }
    }
}
