#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod native;
#[cfg(windows)]
mod runtime;
#[cfg(windows)]
mod updates;
#[cfg(windows)]
mod window;

fn main() {
    #[cfg(windows)]
    if let Err(error) = runtime::main() {
        eprintln!("Nolae could not complete the operation: {error}");
        std::process::exit(1);
    }
    #[cfg(not(windows))]
    {
        eprintln!("This release supports Windows only");
        std::process::exit(1);
    }
}
