use clap::Parser;
use procsuf::opt::{ArgOutputFormat, Opt};

#[test]
fn library_exposes_cli_options() {
    let opt = Opt::try_parse_from(["procsuf", "--format", "json", "needle"])
        .expect("public Opt should parse through the library crate");

    assert_eq!(opt.output_format, Some(ArgOutputFormat::Json));
    assert_eq!(opt.keyword, ["needle"]);
}

#[test]
fn library_exposes_cli_runner() {
    let runner: fn() -> anyhow::Result<()> = procsuf::run;
    let _ = runner;
}
