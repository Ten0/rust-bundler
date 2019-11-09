extern crate bundler;

fn main() {
    let mut args = std::env::args().skip(1);
    let path_to_project = args.next().unwrap_or_else(|| {
        eprintln!("Usage: bundle path/to/project");
        std::process::exit(1);
    });
    let write_code_to = args.next();
    let code = bundler::bundle(&path_to_project);
    if let Some(write_code_to) = write_code_to {
        let write_to_full_path = format!(
            "{}/{}",
            path_to_project.trim_end_matches('/'),
            write_code_to
        );
        eprintln!("Writing to {}", &write_to_full_path);
        std::fs::write(write_to_full_path, code).expect("Could not write output to file");
    } else {
        println!("{}", code);
    }
}
