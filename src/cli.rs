//! Minimal command-line parsing: options + positional `*.lbdb` files.
//!
//! Hand-rolled (no dependency): `bugscope [OPTIONS] [--] [FILE]...`
//! where every positional arg is interpreted as a LadybugDB file to open.

use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct CliOptions {
    /// Explicit `.lbdb` files to open (positional args, in order).
    pub files: Vec<PathBuf>,
    /// Override the directory scanned for databases (`--dir`).
    pub db_dir: Option<PathBuf>,
    /// Override the edge-scan limit (`--limit`).
    pub limit: Option<usize>,
    /// Start in schema mode (`--schema`).
    pub schema_mode: bool,
}

pub const HELP: &str = "\
Usage: bugscope [OPTIONS] [--] [FILE]...

Native graph viewer for LadybugDB files.

Positional arguments:
  [FILE]...   .lbdb files to open; the first one is loaded at startup

Options:
  --dir <PATH>    Directory to scan for .lbdb files (default: current dir)
  --limit <N>     Max edges to load per database (default: 10000)
  --schema        Start in schema view instead of the data graph
  -h, --help      Print this help and exit
  -V, --version   Print the version and exit
";

/// Parse `args` (including argv[0]). Returns `None` when the process should
/// exit immediately (`--help`/`--version` already printed); `Err` otherwise.
pub fn parse_cli(args: &[String]) -> Result<Option<CliOptions>, String> {
    let mut opts = CliOptions::default();
    let mut positional_only = false;
    let mut it = args.iter().skip(1).peekable();
    while let Some(arg) = it.next() {
        if !positional_only && arg == "--" {
            positional_only = true;
            continue;
        }
        if !positional_only && arg.starts_with("--") {
            match arg.as_str() {
                "--help" => {
                    print!("{HELP}");
                    return Ok(None);
                }
                "--version" => {
                    println!("bugscope {}", env!("CARGO_PKG_VERSION"));
                    return Ok(None);
                }
                "--schema" => opts.schema_mode = true,
                "--dir" => {
                    let v = it.next().ok_or("--dir requires a path")?;
                    opts.db_dir = Some(PathBuf::from(v));
                }
                "--limit" => {
                    let v = it.next().ok_or("--limit requires a number")?;
                    opts.limit = Some(
                        v.parse::<usize>()
                            .map_err(|_| format!("--limit requires a number, got {v:?}"))?,
                    );
                }
                other => return Err(format!("unknown option {other:?}\n{HELP}")),
            }
            continue;
        }
        if !positional_only && arg == "-h" {
            print!("{HELP}");
            return Ok(None);
        }
        if !positional_only && arg == "-V" {
            println!("bugscope {}", env!("CARGO_PKG_VERSION"));
            return Ok(None);
        }
        if !positional_only && arg.starts_with('-') && arg.len() > 1 {
            return Err(format!("unknown option {arg:?}\n{HELP}"));
        }
        opts.files.push(PathBuf::from(arg));
    }
    Ok(Some(opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn positionals_become_files() {
        let opts = parse_cli(&args(&["bugscope", "a.lbdb", "b.lbdb"]))
            .unwrap()
            .unwrap();
        assert_eq!(opts.files, vec![PathBuf::from("a.lbdb"), PathBuf::from("b.lbdb")]);
        assert!(!opts.schema_mode);
    }

    #[test]
    fn options_and_separator() {
        let opts = parse_cli(&args(&[
            "bugscope", "--dir", "/tmp", "--limit", "500", "--schema", "--", "--weird.lbdb",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(opts.db_dir, Some(PathBuf::from("/tmp")));
        assert_eq!(opts.limit, Some(500));
        assert!(opts.schema_mode);
        assert_eq!(opts.files, vec![PathBuf::from("--weird.lbdb")]);
    }

    #[test]
    fn rejects_unknown_and_bad_values() {
        assert!(parse_cli(&args(&["bugscope", "--bogus"])).is_err());
        assert!(parse_cli(&args(&["bugscope", "--limit"])).is_err());
        assert!(parse_cli(&args(&["bugscope", "--limit", "abc"])).is_err());
    }
}
