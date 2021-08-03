use anyhow::{Context as _, Error};
use camino::Utf8PathBuf;
use structopt::StructOpt;
use tracing_subscriber::filter::LevelFilter;

fn setup_logger(json: bool, log_level: LevelFilter) -> Result<(), Error> {
    let mut env_filter = tracing_subscriber::EnvFilter::from_default_env();

    // If a user specifies a log level, we assume it only pertains to cargo_fetcher,
    // if they want to trace other crates they can use the RUST_LOG env approach
    env_filter = env_filter.add_directive(format!("xwin={}", log_level).parse()?);

    let subscriber = tracing_subscriber::FmtSubscriber::builder().with_env_filter(env_filter);

    if json {
        tracing::subscriber::set_global_default(subscriber.json().finish())
            .context("failed to set default subscriber")?;
    } else {
        tracing::subscriber::set_global_default(subscriber.finish())
            .context("failed to set default subscriber")?;
    }

    Ok(())
}

#[derive(StructOpt)]
pub enum Command {
    /// Displays a summary of the packages that would be downloaded
    List,
    /// Downloads all the selected packages that aren't already present in
    /// the download cache
    Download,
    /// Unpacks all of the downloaded packages to disk
    Unpack,
    /// Fixes the packages to prune unneeded files and add symlinks to address
    /// file casing issues and then packs the final artifacts into directories
    /// or tarballs
    Pack,
}

const ARCHES: &[&str] = &["x86", "x86_64", "aarch", "aarch64"];
const VARIANTS: &[&str] = &["desktop", "onecore", "store", "spectre"];

fn parse_level(s: &str) -> Result<LevelFilter, Error> {
    s.parse::<LevelFilter>()
        .map_err(|_| anyhow::anyhow!("failed to parse level '{}'", s))
}

#[derive(StructOpt)]
pub struct Args {
    /// Doesn't display prompt to accept the license
    #[structopt(long, env = "XWIN_ACCEPT_LICENSE")]
    accept_license: bool,
    #[structopt(
        short = "L",
        long = "log-level",
        default_value = "info",
        parse(try_from_str = parse_level),
        long_help = "The log level for messages, only log messages at or above the level will be emitted.

Possible values:
* off
* error
* warn
* info (default)
* debug
* trace"
    )]
    level: LevelFilter,
    /// Output log messages as json
    #[structopt(long)]
    json: bool,
    /// If set, will use a temporary directory for all files used for creating
    /// the archive and deleted upon exit, otherwise, all downloaded files
    /// are kept in the current working directory and won't be retrieved again
    #[structopt(long)]
    temp: bool,
    /// Specifies the cache directory used to persist downloaded items to disk.
    /// Defaults to ./.xwin-cache if not specified.
    #[structopt(long)]
    cache_dir: Option<Utf8PathBuf>,
    /// The version to retrieve, can either be a major version of 15 or 16, or
    /// a "<major>.<minor>" version.
    #[structopt(long, default_value = "16")]
    version: String,
    /// The product channel to use.
    #[structopt(long, default_value = "release")]
    channel: String,
    /// The architectures to include
    #[structopt(
        long,
        possible_values(ARCHES),
        use_delimiter = true,
        default_value = "x86_64"
    )]
    arch: Vec<xwin::Arch>,
    /// The variants to include
    #[structopt(
        long,
        possible_values(VARIANTS),
        use_delimiter = true,
        default_value = "desktop"
    )]
    variant: Vec<xwin::Variant>,
    #[structopt(subcommand)]
    cmd: Command,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Error> {
    let args = Args::from_args();
    setup_logger(args.json, args.level)?;

    let ctx = if args.temp {
        xwin::Ctx::with_temp()?
    } else {
        let cache_dir = match args.cache_dir {
            Some(cd) => cd,
            None => {
                let mut cwd = Utf8PathBuf::from_path_buf(
                    std::env::current_dir().context("unable to retrieve cwd")?,
                )
                .map_err(|pb| {
                    anyhow::anyhow!("cache-dir {} is not a valid utf-8 path", pb.display())
                })?;
                cwd.push(".xwin-cache");
                cwd
            }
        };
        xwin::Ctx::with_dir(cache_dir)?
    };

    let ctx = std::sync::Arc::new(ctx);

    let pkg_manifest = xwin::get_pkg_manifest(&ctx, &args.version, &args.channel).await?;

    let arches = args.arch.into_iter().fold(0, |acc, arch| acc | arch as u32);
    let variants = args
        .variant
        .into_iter()
        .fold(0, |acc, var| acc | var as u32);

    let pruned = xwin::prune_pkg_list(&pkg_manifest, arches, variants)?;
    let pkgs = &pkg_manifest.packages;

    match args.cmd {
        Command::List => {
            print_packages(&pruned);
        }
        Command::Download => xwin::download(ctx, pkgs, pruned).await?,
        Command::Unpack => {
            xwin::download(ctx.clone(), pkgs, pruned.clone()).await?;
            xwin::unpack(ctx, pruned).await?;
        }
        Command::Pack => {
            xwin::download(ctx.clone(), pkgs, pruned.clone()).await?;
            xwin::unpack(ctx.clone(), pruned.clone()).await?;
            //xwin::pack(ctx, pruned).await?;
        }
    }

    Ok(())
}

fn print_packages(payloads: &[xwin::Payload]) {
    use cli_table::{format::Justify, Cell, Style, Table};

    let (dl, install) = payloads.iter().fold((0, 0), |(dl, install), payload| {
        (
            dl + payload.size,
            install + payload.install_size.unwrap_or_default(),
        )
    });

    let totals = vec![
        "Total".cell().bold(true).justify(Justify::Right),
        "".cell(),
        "".cell(),
        indicatif::HumanBytes(dl).cell().bold(true),
        indicatif::HumanBytes(install).cell().bold(true),
    ];

    let table = payloads
        .iter()
        .map(|payload| {
            vec![
                payload.filename.clone().cell().justify(Justify::Right),
                payload
                    .target_arch
                    .map(|a| a.to_string())
                    .unwrap_or_default()
                    .cell(),
                payload
                    .variant
                    .map(|v| v.to_string())
                    .unwrap_or_default()
                    .cell(),
                indicatif::HumanBytes(payload.size).cell(),
                indicatif::HumanBytes(payload.install_size.unwrap_or_default()).cell(),
            ]
        })
        .chain(std::iter::once(totals))
        .collect::<Vec<_>>()
        .table()
        .title(vec![
            "Name".cell(),
            "Target".cell(),
            "Variant".cell(),
            "Download Size".cell(),
            "Install Size".cell(),
        ]);

    let _ = cli_table::print_stdout(table);
}
