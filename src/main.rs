mod column;
mod columns;
mod config;
mod opt;
mod process;
mod query;
mod search_regex;
mod style;
mod term_info;
mod util;
mod view;
mod watcher;

use crate::column::Column;
use crate::columns::*;
use crate::config::*;
use crate::opt::*;
use crate::query::{JqTransform, WhereFilter};
use crate::util::{adjust, get_theme, has_regex_syntax, lap};
use crate::view::View;
use crate::watcher::Watcher;
use anyhow::{Context, Error};
use clap::{CommandFactory, Parser};
use clap_mangen::Man;
use console::Term;
use once_cell::sync::Lazy;
use std::cmp;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, stdout};
use std::path::PathBuf;
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

// ---------------------------------------------------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------------------------------------------------

static KIND_NAMES_LOWER: Lazy<Vec<String>> = Lazy::new(|| {
    KIND_LIST
        .iter()
        .map(|(_, (name, _))| name.to_lowercase())
        .collect()
});

fn command_with_kind_values() -> clap::Command {
    let kind_values: Vec<clap::builder::PossibleValue> = KIND_LIST
        .iter()
        .zip(KIND_NAMES_LOWER.iter())
        .map(|((_, (_, desc)), lower)| {
            clap::builder::PossibleValue::new(lower.as_str()).help(*desc)
        })
        .collect();
    let parser = clap::builder::PossibleValuesParser::new(kind_values);
    Opt::command()
        .mut_arg("sorta", |a| a.value_parser(parser.clone()))
        .mut_arg("sortd", |a| a.value_parser(parser.clone()))
        .mut_arg("insert", |a| a.value_parser(parser.clone()))
        .mut_arg("only", |a| a.value_parser(parser))
}

fn select_config_path(
    explicit: Option<PathBuf>,
    procsuf_paths: impl IntoIterator<Item = Option<PathBuf>>,
    procs_paths: impl IntoIterator<Item = Option<PathBuf>>,
) -> Option<PathBuf> {
    explicit
        .or_else(|| procsuf_paths.into_iter().flatten().next())
        .or_else(|| procs_paths.into_iter().flatten().next())
}

fn get_config(opt: &Opt) -> Result<Config, Error> {
    let procsuf_dot_cfg_path = directories::BaseDirs::new()
        .map(|base| base.home_dir().join(".procsuf.toml"))
        .filter(|path| path.exists());
    let procsuf_app_cfg_path = directories::ProjectDirs::from("com.github", "ijnebzor", "procsuf")
        .map(|proj| proj.preference_dir().join("config.toml"))
        .filter(|path| path.exists());
    let procsuf_xdg_cfg_path = directories::BaseDirs::new()
        .map(|base| {
            base.home_dir()
                .join(".config")
                .join("procsuf")
                .join("config.toml")
        })
        .filter(|path| path.exists());
    let procsuf_etc_cfg_path = PathBuf::from("/etc/procsuf/procsuf.toml");
    let procsuf_etc_cfg_path = procsuf_etc_cfg_path
        .exists()
        .then_some(procsuf_etc_cfg_path);

    let procs_dot_cfg_path = directories::BaseDirs::new()
        .map(|base| base.home_dir().join(".procs.toml"))
        .filter(|path| path.exists());
    let procs_app_cfg_path = directories::ProjectDirs::from("com.github", "dalance", "procs")
        .map(|proj| proj.preference_dir().join("config.toml"))
        .filter(|path| path.exists());
    let procs_xdg_cfg_path = directories::BaseDirs::new()
        .map(|base| {
            base.home_dir()
                .join(".config")
                .join("procs")
                .join("config.toml")
        })
        .filter(|path| path.exists());
    let procs_etc_cfg_path = PathBuf::from("/etc/procs/procs.toml");
    let procs_etc_cfg_path = procs_etc_cfg_path.exists().then_some(procs_etc_cfg_path);
    let cfg_path = select_config_path(
        opt.load_config.clone(),
        [
            procsuf_dot_cfg_path,
            procsuf_app_cfg_path,
            procsuf_xdg_cfg_path,
            procsuf_etc_cfg_path,
        ],
        [
            procs_dot_cfg_path,
            procs_app_cfg_path,
            procs_xdg_cfg_path,
            procs_etc_cfg_path,
        ],
    );

    let config: Config = if let Some(path) = cfg_path {
        let mut f = fs::File::open(&path).context(format!("failed to open file ({path:?})"))?;
        let mut s = String::new();
        f.read_to_string(&mut s)
            .context(format!("failed to read file ({path:?})"))?;
        let c = toml::from_str(&s);
        check_old_config(&s, c).context(format!("failed to parse toml ({path:?})"))?
    } else {
        toml::from_str(CONFIG_DEFAULT).unwrap()
    };

    match opt.use_config {
        Some(BuiltinConfig::Default) => Ok(toml::from_str(CONFIG_DEFAULT).unwrap()),
        Some(BuiltinConfig::Large) => Ok(toml::from_str(CONFIG_LARGE).unwrap()),
        None => Ok(config),
    }
}

fn check_old_config(s: &str, config: Result<Config, toml::de::Error>) -> Result<Config, Error> {
    match config {
        Ok(x) => Ok(x),
        Err(x) => {
            if s.contains("Color256") {
                let err: Error = x.into();
                let err = err.context("\"Color256\" keyword for 8bit color is obsolete. Please see https://github.com/ijnebzor/procsuf#color-list");
                Err(err)
            } else {
                Err(x.into())
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------------------------------------------------

fn main() {
    let err = Term::stderr();

    if let Err(x) = run() {
        let mut cause = x.chain();
        let _ = err.write_line(&format!(
            "{} {}",
            console::style("error:").red().bold(),
            cause.next().unwrap()
        ));

        for x in cause {
            let _ = err.write_line(&format!("  {} {}", console::style("caused by:").red(), x));
        }

        std::process::exit(1);
    }
}

fn run() -> Result<(), Error> {
    let mut opt: Opt = Parser::parse();
    opt.watch_mode = opt.watch || opt.watch_interval.is_some();
    validate_search_args(&opt)?;
    validate_output_args(&opt)?;

    if opt.gen_config {
        run_gen_config()
    } else if opt.list {
        run_list();
        Ok(())
    } else if let Some(shell) = opt.gen_completion {
        gen_completion(shell, "./", &mut command_with_kind_values())
    } else if let Some(shell) = opt.gen_completion_out {
        clap_complete::generate(
            shell,
            &mut command_with_kind_values(),
            "procsuf",
            &mut stdout(),
        );
        Ok(())
    } else if opt.gen_man_page {
        let cmd = command_with_kind_values();
        let man = Man::new(cmd);
        man.render(&mut stdout())?;
        Ok(())
    } else {
        let config = get_config(&opt)?;
        if opt.watch_mode {
            let interval = match opt.watch_interval {
                Some(n) => (n * 1000.0).round() as u64,
                None => 1000,
            };
            run_watch(&mut opt, &config, interval)
        } else {
            run_default(&mut opt, &config)
        }
    }
}

fn run_gen_config() -> Result<(), Error> {
    let config: Config = toml::from_str(CONFIG_DEFAULT).unwrap();
    let toml = toml::to_string(&config)?;
    println!("{toml}");
    Ok(())
}

fn run_list() {
    let mut width = 0;
    let mut list = Vec::new();
    let mut desc = HashMap::new();
    for (_, (v, d)) in KIND_LIST.iter() {
        list.push(v);
        desc.insert(v, d);
        width = cmp::max(width, UnicodeWidthStr::width(*v));
    }

    println!("Column kind list:");
    for l in list {
        println!(
            "  {}: {}",
            adjust(l, width, &ConfigColumnAlign::Left),
            desc[l]
        );
    }
}

fn run_watch(opt: &mut Opt, config: &Config, interval: u64) -> Result<(), Error> {
    Watcher::start(opt, config, interval)
}

fn run_default(opt: &mut Opt, config: &Config) -> Result<(), Error> {
    let mut time = Instant::now();

    let where_filter = opt
        .where_expr
        .as_deref()
        .map(WhereFilter::compile)
        .transpose()?;
    let jq_transform = opt
        .jq_filter
        .as_deref()
        .map(JqTransform::compile)
        .transpose()?;
    let theme = get_theme(opt, config);

    let mut view = View::new(opt, config, false)?;

    if opt.debug {
        lap(&mut time, "Info: View::new");
    }

    view.filter(opt, config, 1, where_filter.as_ref())?;

    if opt.debug {
        lap(&mut time, "Info: view.filter");
    }

    view.adjust(config, &HashMap::new());

    if opt.debug {
        lap(&mut time, "Info: view.adjust");
    }

    if let Some(transform) = &jq_transform {
        view.display_jq(transform, opt.pretty)?;
    } else {
        view.display(opt, config, &theme)?;
    }

    if opt.debug {
        lap(&mut time, "Info: view.display");
    }

    Ok(())
}

fn validate_search_args(opt: &Opt) -> Result<(), Error> {
    if opt.regex && opt.keyword.len() > 1 {
        anyhow::bail!("--regex accepts a single PATTERN argument");
    }
    if opt.smart && opt.keyword.len() > 1 && opt.keyword.iter().any(|k| has_regex_syntax(k)) {
        anyhow::bail!("--smart supports a single PATTERN when regex syntax is detected");
    }
    Ok(())
}

fn validate_output_args(opt: &Opt) -> Result<(), Error> {
    if opt.pretty
        && !opt.json
        && opt.jq_filter.is_none()
        && !matches!(opt.output_format, Some(ArgOutputFormat::Json))
    {
        anyhow::bail!("--pretty requires --json, --format json, or --jq");
    }
    if opt.jq_filter.is_some() {
        if opt.watch_mode {
            anyhow::bail!("--jq cannot be combined with watch mode");
        }
        if opt.json {
            anyhow::bail!(
                "--jq cannot be combined with legacy --json; omit --json or use --format json"
            );
        }
        match opt.output_format {
            Some(ArgOutputFormat::Jsonl) => {
                anyhow::bail!("--jq cannot be combined with --format jsonl");
            }
            Some(ArgOutputFormat::Table) => {
                anyhow::bail!("--jq cannot be combined with --format table");
            }
            Some(ArgOutputFormat::Json) | None => {}
        }
        if opt.list
            || opt.gen_config
            || opt.gen_completion.is_some()
            || opt.gen_completion_out.is_some()
            || opt.gen_man_page
        {
            anyhow::bail!("--jq is only available for process-list output");
        }
    }
    if opt.watch_mode && opt.output_format == Some(ArgOutputFormat::Jsonl) {
        anyhow::bail!("--format jsonl cannot be combined with watch mode");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procsuf_config_takes_precedence_over_upstream_config() {
        let procsuf = PathBuf::from("procsuf/config.toml");
        let procs = PathBuf::from("procs/config.toml");

        let selected = select_config_path(None, [None, Some(procsuf.clone())], [Some(procs)]);

        assert_eq!(selected, Some(procsuf));
    }

    #[test]
    fn upstream_config_is_used_only_without_a_procsuf_config() {
        let procs = PathBuf::from("procs/config.toml");

        let selected = select_config_path(None, [None, None], [None, Some(procs.clone())]);

        assert_eq!(selected, Some(procs));
    }

    #[test]
    fn explicit_config_takes_precedence_over_discovered_configs() {
        let explicit = PathBuf::from("custom/config.toml");

        let selected = select_config_path(
            Some(explicit.clone()),
            [Some(PathBuf::from("procsuf/config.toml"))],
            [Some(PathBuf::from("procs/config.toml"))],
        );

        assert_eq!(selected, Some(explicit));
    }

    #[test]
    fn test_run() {
        let mut config: Config = toml::from_str(CONFIG_DEFAULT).unwrap();
        config.pager.mode = ConfigPagerMode::Disable;
        config.display.theme = ConfigTheme::Dark;

        let args = ["procsuf"];
        let mut opt = Opt::parse_from(args.iter());
        let ret = run_default(&mut opt, &config);
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_search() {
        let mut config: Config = toml::from_str(CONFIG_DEFAULT).unwrap();
        config.pager.mode = ConfigPagerMode::Disable;
        config.display.theme = ConfigTheme::Dark;

        let args = ["procsuf", "root"];
        let mut opt = Opt::parse_from(args.iter());
        let ret = run_default(&mut opt, &config);
        assert!(ret.is_ok());

        let args = ["procsuf", "1"];
        let mut opt = Opt::parse_from(args.iter());
        let ret = run_default(&mut opt, &config);
        assert!(ret.is_ok());

        let args = ["procsuf", "--or", "root", "1"];
        let mut opt = Opt::parse_from(args.iter());
        let ret = run_default(&mut opt, &config);
        assert!(ret.is_ok());

        let args = ["procsuf", "--and", "root", "1"];
        let mut opt = Opt::parse_from(args.iter());
        let ret = run_default(&mut opt, &config);
        assert!(ret.is_ok());

        let args = ["procsuf", "--nor", "root", "1"];
        let mut opt = Opt::parse_from(args.iter());
        let ret = run_default(&mut opt, &config);
        assert!(ret.is_ok());

        let args = ["procsuf", "--nand", "root", "1"];
        let mut opt = Opt::parse_from(args.iter());
        let ret = run_default(&mut opt, &config);
        assert!(ret.is_ok());

        config.search.nonnumeric_search = ConfigSearchKind::Exact;
        config.search.numeric_search = ConfigSearchKind::Partial;
        let args = ["procsuf", "root", "1"];
        let mut opt = Opt::parse_from(args.iter());
        let ret = run_default(&mut opt, &config);
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_gen_config() {
        let ret = run_gen_config();
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_without_truncate() {
        let mut config: Config = toml::from_str(CONFIG_DEFAULT).unwrap();
        config.display.cut_to_terminal = false;
        config.display.theme = ConfigTheme::Dark;

        let args = ["procsuf"];
        let mut opt = Opt::parse_from(args.iter());
        config.pager.mode = ConfigPagerMode::Disable;
        let ret = run_default(&mut opt, &config);
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_insert() {
        let mut config: Config = toml::from_str(CONFIG_DEFAULT).unwrap();
        config.pager.mode = ConfigPagerMode::Disable;
        config.display.theme = ConfigTheme::Dark;

        let args = ["procsuf", "--insert", "ppid"];
        let mut opt = Opt::parse_from(args.iter());
        let ret = run_default(&mut opt, &config);
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_sort() {
        let mut config: Config = toml::from_str(CONFIG_DEFAULT).unwrap();
        config.pager.mode = ConfigPagerMode::Disable;
        config.display.theme = ConfigTheme::Dark;

        let args = ["procsuf", "--sorta", "cpu"];
        let mut opt = Opt::parse_from(args.iter());
        let ret = run_default(&mut opt, &config);
        assert!(ret.is_ok());

        let args = ["procsuf", "--sortd", "cpu"];
        let mut opt = Opt::parse_from(args.iter());
        let ret = run_default(&mut opt, &config);
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_tree() {
        let mut config: Config = toml::from_str(CONFIG_DEFAULT).unwrap();
        config.pager.mode = ConfigPagerMode::Disable;
        config.display.theme = ConfigTheme::Dark;

        let args = ["procsuf", "--tree"];
        let mut opt = Opt::parse_from(args.iter());
        let ret = run_default(&mut opt, &config);
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_all() {
        let mut config: Config = toml::from_str(CONFIG_ALL).unwrap();
        config.pager.mode = ConfigPagerMode::Disable;
        config.display.theme = ConfigTheme::Dark;

        let _tcp = std::net::TcpListener::bind("127.0.0.1:10000");
        let _udp = std::net::UdpSocket::bind("127.0.0.1:10000");

        let args = ["procsuf"];
        let mut opt = Opt::parse_from(args.iter());
        let ret = run_default(&mut opt, &config);
        assert!(ret.is_ok());
    }

    #[test]
    fn test_run_use_config() {
        let mut config: Config = toml::from_str(CONFIG_DEFAULT).unwrap();
        config.pager.mode = ConfigPagerMode::Disable;
        config.display.theme = ConfigTheme::Dark;

        let args = ["procsuf", "--use-config", "large"];
        let mut opt = Opt::parse_from(args.iter());
        let ret = run_default(&mut opt, &config);
        assert!(ret.is_ok());
    }

    #[test]
    fn test_validate_output_args() {
        let opt = Opt::parse_from(["procsuf", "--json", "--pretty"]);
        assert!(validate_output_args(&opt).is_ok());

        let opt = Opt::parse_from(["procsuf", "--format", "json", "--pretty"]);
        assert!(validate_output_args(&opt).is_ok());

        let opt = Opt::parse_from(["procsuf", "--pretty"]);
        assert!(validate_output_args(&opt).is_err());

        let opt = Opt::parse_from(["procsuf", "--format", "jsonl", "--pretty"]);
        assert!(validate_output_args(&opt).is_err());

        let mut opt = Opt::parse_from(["procsuf", "--format", "jsonl", "--watch"]);
        opt.watch_mode = true;
        assert!(validate_output_args(&opt).is_err());

        let opt = Opt::parse_from(["procsuf", "--jq", ".", "--pretty"]);
        assert!(validate_output_args(&opt).is_ok());

        let opt = Opt::parse_from(["procsuf", "--jq", ".", "--format", "json"]);
        assert!(validate_output_args(&opt).is_ok());

        let mut opt = Opt::parse_from(["procsuf", "--jq", ".", "--watch"]);
        opt.watch_mode = true;
        assert_eq!(
            validate_output_args(&opt).unwrap_err().to_string(),
            "--jq cannot be combined with watch mode"
        );

        for args in [
            ["procsuf", "--jq", ".", "--format", "jsonl"],
            ["procsuf", "--jq", ".", "--format", "table"],
        ] {
            assert!(validate_output_args(&Opt::parse_from(args)).is_err());
        }

        let opt = Opt::parse_from(["procsuf", "--jq", ".", "--json"]);
        assert!(validate_output_args(&opt).is_err());

        let opt = Opt::parse_from(["procsuf", "--jq", ".", "--list"]);
        assert!(validate_output_args(&opt).is_err());
    }

    #[test]
    fn test_legacy_json_conflicts_with_canonical_format() {
        assert!(Opt::try_parse_from(["procsuf", "--json", "--format", "json"]).is_err());
    }
}
