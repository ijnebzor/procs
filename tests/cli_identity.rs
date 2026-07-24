use std::process::{Command, Output};

fn run_procsuf(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_procsuf"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn help_and_version_use_the_procsuf_identity() {
    let help = run_procsuf(&["--help"]);
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("procs unfucked."));
    assert!(help.contains("Usage: procsuf"));
    assert!(help.contains("https://github.com/ijnebzor/procsuf#configuration"));

    let version = run_procsuf(&["--version"]);
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap().trim(),
        "procsuf 0.1.0"
    );
}

#[test]
fn generated_completion_uses_the_procsuf_command() {
    let completion = run_procsuf(&["--gen-completion-out", "bash"]);
    assert!(completion.status.success());
    let completion = String::from_utf8(completion.stdout).unwrap();
    assert!(completion.contains("_procsuf"));
    assert!(completion.contains("procsuf"));
}

#[test]
fn generated_man_page_uses_the_procsuf_identity() {
    let man_page = run_procsuf(&["--gen-man-page"]);
    assert!(man_page.status.success());
    let man_page = String::from_utf8(man_page.stdout).unwrap().to_lowercase();
    assert!(man_page.contains(".th procsuf 1"));
    assert!(man_page.contains("procs unfucked."));
}
