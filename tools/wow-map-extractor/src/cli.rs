//! Command line: port of `Usage` and `HandleArgs` from
//! `src/tools/map_extractor/System.cpp`, with the same single-letter switches, the
//! same `atoi` leniency and the same defaults (`CONF_*` globals).

use std::path::PathBuf;

use crate::casc::{LOCALE_NAMES, TOTAL_LOCALES};

/// `enum Extract`.
pub(crate) const EXTRACT_MAP: i32 = 0x1;
pub(crate) const EXTRACT_DBC: i32 = 0x2;
pub(crate) const EXTRACT_CAMERA: i32 = 0x4;
pub(crate) const EXTRACT_GT: i32 = 0x8;
pub(crate) const EXTRACT_ALL: i32 = EXTRACT_MAP | EXTRACT_DBC | EXTRACT_CAMERA | EXTRACT_GT;

/// Extractor options (`input_path`, `output_path` and the `CONF_*` globals).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Config {
    pub(crate) input_path: PathBuf,
    pub(crate) output_path: PathBuf,
    /// `CONF_extract`.
    pub(crate) extract: i32,
    /// `CONF_allow_float_to_int`.
    pub(crate) allow_float_to_int: bool,
    /// `CONF_Locale`: `1 << LocaleConstant`, 0 = every installed locale.
    pub(crate) locale: u32,
    /// `CONF_Product`.
    pub(crate) product: String,
    /// `CONF_Region`.
    pub(crate) region: String,
    /// `CONF_UseRemoteCasc`.
    pub(crate) use_remote_casc: bool,
}

impl Config {
    /// Defaults; `input_path`/`output_path` are the current directory in `main`.
    pub(crate) fn new(current_dir: PathBuf) -> Self {
        Self {
            input_path: current_dir.clone(),
            output_path: current_dir,
            extract: EXTRACT_ALL,
            allow_float_to_int: true,
            locale: 0,
            product: "wow_classic".to_owned(),
            region: "eu".to_owned(),
            use_remote_casc: false,
        }
    }
}

/// `Usage(prg)` text (the C++ prints it and calls `exit(1)`).
pub(crate) fn usage(prg: &str) -> String {
    format!(
        "Usage:\n\
         {prg} -[var] [value]\n\
         -i set input path\n\
         -o set output path\n\
         -e extract only MAP(1)/DBC(2)/Camera(4)/gt(8) - standard: all(15)\n\
         -f height stored as int (less map size but lost some accuracy) 1 by default\n\
         -l dbc locale\n\
         -p which installed product to open (wow/wowt/wow_beta)\n\
         -c use remote casc\n\
         -r set remote casc region - standard: eu\n\
         Example: {prg} -f 0 -i \"c:\\games\\game\"\n"
    )
}

/// C `atoi`: optional whitespace and sign, then leading decimal digits; 0 otherwise.
pub(crate) fn atoi(s: &str) -> i32 {
    let s = s.trim_start();
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut value: i64 = 0;
    for b in digits.bytes().take_while(u8::is_ascii_digit) {
        value = (value * 10 + i64::from(b - b'0')).min(i64::from(u32::MAX) + 1);
    }
    let value = if negative { -value } else { value };
    value as i32
}

/// `HandleArgs(argc, arg)`. `Err(())` means `Usage(arg[0])` (print + `exit(1)`).
pub(crate) fn handle_args(args: &[String], mut config: Config) -> Result<Config, ()> {
    let mut c = 1;
    while c < args.len() {
        let arg = args[c].as_bytes();
        if arg.first() != Some(&b'-') {
            return Err(());
        }
        let next = args.get(c + 1);
        match arg.get(1) {
            Some(b'i') => match next {
                Some(v) if !v.is_empty() => {
                    config.input_path = PathBuf::from(v);
                    c += 1;
                }
                _ => return Err(()),
            },
            Some(b'o') => match next {
                Some(v) if !v.is_empty() => {
                    config.output_path = PathBuf::from(v);
                    c += 1;
                }
                _ => return Err(()),
            },
            Some(b'f') => {
                let v = next.ok_or(())?;
                config.allow_float_to_int = atoi(v) != 0;
                c += 1;
            }
            Some(b'e') => {
                let v = next.ok_or(())?;
                config.extract = atoi(v);
                c += 1;
                if !(config.extract > 0 && config.extract <= EXTRACT_ALL) {
                    return Err(());
                }
            }
            Some(b'l') => {
                let v = next.ok_or(())?;
                for (i, name) in LOCALE_NAMES.iter().enumerate().take(TOTAL_LOCALES) {
                    if v == name {
                        config.locale = 1 << i;
                    }
                }
                c += 1;
            }
            Some(b'p') => match next {
                Some(v) if !v.is_empty() => {
                    config.product.clone_from(v);
                    c += 1;
                }
                _ => return Err(()),
            },
            Some(b'c') => {
                let v = next.ok_or(())?;
                config.use_remote_casc = atoi(v) != 0;
                c += 1;
            }
            Some(b'r') => match next {
                Some(v) if !v.is_empty() => {
                    config.region.clone_from(v);
                    c += 1;
                }
                _ => return Err(()),
            },
            Some(b'h') => return Err(()),
            _ => {}
        }
        c += 1;
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Config, ()> {
        let args: Vec<String> = std::iter::once("map_extractor")
            .chain(args.iter().copied())
            .map(String::from)
            .collect();
        handle_args(&args, Config::new(PathBuf::from("/cwd")))
    }

    #[test]
    fn defaults() {
        let c = parse(&[]).unwrap();
        assert_eq!(c.input_path, PathBuf::from("/cwd"));
        assert_eq!(c.output_path, PathBuf::from("/cwd"));
        assert_eq!(c.extract, 15);
        assert!(c.allow_float_to_int);
        assert_eq!(c.locale, 0);
        assert_eq!(c.product, "wow_classic");
        assert_eq!(c.region, "eu");
        assert!(!c.use_remote_casc);
    }

    #[test]
    fn all_switches() {
        let c = parse(&[
            "-i",
            "/wow",
            "-o",
            "/out",
            "-e",
            "3",
            "-f",
            "0",
            "-l",
            "deDE",
            "-p",
            "wow_classic_era",
            "-c",
            "1",
            "-r",
            "us",
        ])
        .unwrap();
        assert_eq!(c.input_path, PathBuf::from("/wow"));
        assert_eq!(c.output_path, PathBuf::from("/out"));
        assert_eq!(c.extract, 3);
        assert!(!c.allow_float_to_int);
        assert_eq!(c.locale, 1 << 3);
        assert_eq!(c.product, "wow_classic_era");
        assert!(c.use_remote_casc);
        assert_eq!(c.region, "us");
    }

    #[test]
    fn usage_cases() {
        assert!(parse(&["x"]).is_err());
        assert!(parse(&["-i"]).is_err());
        assert!(parse(&["-i", ""]).is_err());
        assert!(parse(&["-e", "0"]).is_err());
        assert!(parse(&["-e", "16"]).is_err());
        assert!(parse(&["-e", "abc"]).is_err());
        assert!(parse(&["-h"]).is_err());
        assert!(parse(&["-f"]).is_err());
    }

    #[test]
    fn lenient_cases() {
        // Unknown locale keeps "all"; unknown switch is ignored (its value then fails).
        assert_eq!(parse(&["-l", "xxXX"]).unwrap().locale, 0);
        assert!(parse(&["-z"]).is_ok());
        assert!(parse(&["-z", "1"]).is_err());
        assert_eq!(parse(&["-e", "8abc"]).unwrap().extract, 8);
        assert!(
            parse(&["-f", "yes"])
                .map(|c| !c.allow_float_to_int)
                .unwrap()
        );
    }

    #[test]
    fn atoi_like_c() {
        assert_eq!(atoi("  42x"), 42);
        assert_eq!(atoi("-7"), -7);
        assert_eq!(atoi("+3"), 3);
        assert_eq!(atoi(""), 0);
        assert_eq!(atoi("x1"), 0);
    }
}
