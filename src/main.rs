extern crate bundler;

use std::collections::HashSet;

fn main() {
	let mut args = std::env::args().skip(1).peekable();
	let write_code_to = if args.peek().unwrap_or_else(|| usage()) == "-o" {
		Some(args.nth(1).unwrap_or_else(|| usage()))
	} else {
		None
	};
	let path_to_project = args.next().unwrap_or_else(|| usage());
	let mut skip_extern_crate_expansion: HashSet<String> = args.collect();
	skip_extern_crate_expansion.insert("std".to_owned());
	skip_extern_crate_expansion.insert("core".to_owned());
	let code = bundler::bundle(&path_to_project, &skip_extern_crate_expansion);
	if let Some(write_code_to) = write_code_to {
		let write_to_full_path = format!("{}/{}", path_to_project.trim_end_matches('/'), write_code_to);
		eprintln!("Writing to {}", &write_to_full_path);
		std::fs::write(write_to_full_path, code).expect("Could not write output to file");
	} else {
		println!("{}", code);
	}
}

fn usage() -> ! {
	eprintln!("Usage: bundle [-o output] path/to/project excluded_dep1 excluded_dep2");
	std::process::exit(1);
}
