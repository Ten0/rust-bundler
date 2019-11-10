extern crate assert_cli;

use assert_cli::Assert;

#[test]
fn usage() {
    Assert::main_binary()
        .fails()
        .stderr()
        .contains("Usage: bundle")
        .unwrap();
}

#[test]
fn bundle_self() {
    Assert::main_binary()
        .with_args(&[".", "cargo_metadata", "quote", "rustfmt", "syn"])
        .stdout()
        .contains("pub fn bundle<")
        .stdout()
        .contains("let code = bundle(")
        .unwrap();
}
