//! `wow-client-downloader`: command-line downloader of the World of Warcraft
//! Classic 3.4.3.54261 client (`wow_classic`) that writes a local CASC
//! install readable by the game client and by `wow-casc`.
//!
//! A command-line reimplementation of the idea behind blizzget
//! (<https://github.com/d07RiV/blizzget>): because Blizzard's patch server no
//! longer lists this build, a temporary local server ([`server`]) emulates
//! the `versions`/`cdns` endpoints and proxies the CDN (Blizzard first, then
//! the `archive.wow.tools` mirror); the downloader reads everything through
//! it ([`remote`], [`download`]).

mod blte;
mod casc;
mod cdn_index;
mod cli;
mod config;
mod download;
mod encoding;
mod fetch;
mod http;
mod jenkins;
mod manifest;
mod plan;
mod product;
mod progress;
mod remote;
mod root;
mod server;
mod tables;
mod tags;
mod util;

use std::process::ExitCode;

fn run() -> anyhow::Result<()> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match cli::parse(&argv)? {
        cli::Command::Help => {
            print!("{}", cli::USAGE);
            Ok(())
        }
        cli::Command::Download(opts) => download::run(&opts),
        cli::Command::List(net) => download::list(&net),
        cli::Command::Serve(opts) => {
            let mut server_opts =
                server::ServerOptions::new(opts.bind, opts.port, opts.mirror, opts.cache);
            server_opts.verbose = true;
            server::serve(server_opts)
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
