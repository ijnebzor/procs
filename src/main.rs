use console::Term;

fn main() {
    let err = Term::stderr();

    if let Err(error) = procsuf::run() {
        let mut causes = error.chain();
        let _ = err.write_line(&format!(
            "{} {}",
            console::style("error:").red().bold(),
            causes
                .next()
                .expect("an anyhow error always has a root cause")
        ));

        for cause in causes {
            let _ = err.write_line(&format!(
                "  {} {}",
                console::style("caused by:").red(),
                cause
            ));
        }

        std::process::exit(1);
    }
}
