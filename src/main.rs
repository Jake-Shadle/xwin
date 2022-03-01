use anyhow::{Context as _, Error};
use camino::Utf8PathBuf as PathBuf;
use clap::{Parser, Subcommand};
use indicatif as ia;
use tracing_subscriber::filter::LevelFilter;

fn setup_logger(json: bool, log_level: LevelFilter) -> Result<(), Error> {
    let mut env_filter = tracing_subscriber::EnvFilter::from_default_env();

    // If a user specifies a log level, we assume it only pertains to xwin,
    // if they want to trace other crates they can use the RUST_LOG env approach
    env_filter = env_filter.add_directive(format!("xwin={}", log_level).parse()?);

    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr);

    if json {
        tracing::subscriber::set_global_default(subscriber.json().finish())
            .context("failed to set default subscriber")?;
    } else {
        tracing::subscriber::set_global_default(subscriber.finish())
            .context("failed to set default subscriber")?;
    }

    Ok(())
}

#[derive(Subcommand)]
pub enum Command {
    /// Displays a summary of the packages that would be downloaded.
    ///
    /// Note that this is not a full list as the SDK uses MSI files for many
    /// packages, so they would need to be downloaded and inspected to determine
    /// which CAB files must also be downloaded to get the content needed.
    List,
    /// Downloads all the selected packages that aren't already present in
    /// the download cache
    Download,
    /// Unpacks all of the downloaded packages to disk
    Unpack,
    /// Fixes the packages to prune unneeded files and adds symlinks to address
    /// file casing issues and then spalts the final artifacts into directories
    Splat {
        /// The MSVCRT includes (non-redistributable) debug versions of the
        /// various libs that are generally uninteresting to keep for most usage
        #[clap(long)]
        include_debug_libs: bool,
        /// The MSVCRT includes PDB (debug symbols) files for several of the
        /// libraries that are generally uninteresting to keep for most usage
        #[clap(long)]
        include_debug_symbols: bool,
        /// By default, symlinks are added to both the CRT and WindowsSDK to
        /// address casing issues in general usage. For example, if you are
        /// compiling C/C++ code that does `#include <windows.h>`, it will break
        /// on a case-sensitive file system, as the actual path in the WindowsSDK
        /// is `Windows.h`. This also applies even if the C/C++ you are compiling
        /// uses correct casing for all CRT/SDK includes, as the internal headers
        /// also use incorrect casing in most cases.
        #[clap(long)]
        disable_symlinks: bool,
        /// By default, we convert the MS specific `x64`, `arm`, and `arm64`
        /// target architectures to the more canonical `x86_64`, `aarch`, and
        /// `aarch64` of LLVM etc when creating directories/names. Passing this
        /// flag will preserve the MS names for those targets.
        #[clap(long)]
        preserve_ms_arch_notation: bool,
        /// The root output directory. Defaults to `./.xwin-cache/splat` if not
        /// specified.
        #[clap(long)]
        output: Option<PathBuf>,
        /// Copies files from the unpack directory to the splat directory instead
        /// of moving them, which preserves the original unpack directories but
        /// increases overall time and disk usage
        #[clap(long)]
        copy: bool,
        // Splits the CRT and SDK into architecture and variant specific
        // directories. The shared headers in the CRT and SDK are duplicated
        // for each output so that each combination is self-contained.
        // #[clap(long)]
        // isolated: bool,
    },
}

const ARCHES: &[&str] = &["x86", "x86_64", "aarch", "aarch64"];
const VARIANTS: &[&str] = &["desktop", "onecore", /*"store",*/ "spectre"];
const LOG_LEVELS: &[&str] = &["off", "error", "warn", "info", "debug", "trace"];

fn parse_level(s: &str) -> Result<LevelFilter, Error> {
    s.parse::<LevelFilter>()
        .map_err(|_| anyhow::anyhow!("failed to parse level '{}'", s))
}

#[derive(Parser)]
pub struct Args {
    /// Doesn't display the prompt to accept the license
    #[clap(long, env = "XWIN_ACCEPT_LICENSE")]
    accept_license: bool,
    /// The log level for messages, only log messages at or above the level will be emitted.
    #[clap(
        short = 'L',
        long = "log-level",
        default_value = "info",
        parse(try_from_str = parse_level),
        possible_values(LOG_LEVELS),
    )]
    level: LevelFilter,
    /// Output log messages as json
    #[clap(long)]
    json: bool,
    /// If set, will use a temporary directory for all files used for creating
    /// the archive and deleted upon exit, otherwise, all downloaded files
    /// are kept in the `--cache-dir` and won't be retrieved again
    #[clap(long)]
    temp: bool,
    /// Specifies the cache directory used to persist downloaded items to disk.
    /// Defaults to `./.xwin-cache` if not specified.
    #[clap(long)]
    cache_dir: Option<PathBuf>,
    /// Specifies a VS manifest to use from a file, rather than downloading it
    /// from the Microsoft site.
    #[clap(long, conflicts_with_all = &["version", "channel"])]
    manifest: Option<PathBuf>,
    /// The version to retrieve, can either be a major version of 15 or 16, or
    /// a "<major>.<minor>" version.
    #[clap(long, default_value = "16")]
    version: String,
    /// The product channel to use.
    #[clap(long, default_value = "release")]
    channel: String,
    /// The architectures to include
    #[clap(
        long,
        possible_values(ARCHES),
        use_value_delimiter = true,
        default_value = "x86_64"
    )]
    arch: Vec<xwin::Arch>,
    /// The variants to include
    #[clap(
        long,
        possible_values(VARIANTS),
        use_value_delimiter = true,
        default_value = "desktop"
    )]
    variant: Vec<xwin::Variant>,
    #[clap(subcommand)]
    cmd: Command,
}

fn main() -> Result<(), Error> {
    let args = Args::parse();
    setup_logger(args.json, args.level)?;

    if !args.accept_license {
        // The license link is the same for every locale, but we should probably
        // retrieve it from the manifest in the future
        println!("Do you accept the license at https://go.microsoft.com/fwlink/?LinkId=2086102 (yes | no)?");

        let mut accept = String::new();
        std::io::stdin().read_line(&mut accept)?;

        match accept.trim() {
            "yes" => println!("license accepted!"),
            "no" => anyhow::bail!("license not accepted"),
            other => anyhow::bail!("unknown response to license request {}", other),
        }
    }

    let cwd = PathBuf::from_path_buf(std::env::current_dir().context("unable to retrieve cwd")?)
        .map_err(|pb| anyhow::anyhow!("cwd {} is not a valid utf-8 path", pb.display()))?;

    let draw_target = xwin::util::ProgressTarget::Stdout;

    let ctx = if args.temp {
        xwin::Ctx::with_temp(draw_target)?
    } else {
        let cache_dir = match &args.cache_dir {
            Some(cd) => cd.clone(),
            None => cwd.join(".xwin-cache"),
        };
        xwin::Ctx::with_dir(cache_dir, draw_target)?
    };

    let ctx = std::sync::Arc::new(ctx);

    let pkg_manifest = load_manifest(&ctx, &args, draw_target)?;

    let arches = args.arch.into_iter().fold(0, |acc, arch| acc | arch as u32);
    let variants = args
        .variant
        .into_iter()
        .fold(0, |acc, var| acc | var as u32);

    let pruned = xwin::prune_pkg_list(&pkg_manifest, arches, variants)?;

    let op = match args.cmd {
        Command::List => {
            print_packages(&pruned);
            return Ok(());
        }
        Command::Download => xwin::Ops::Download,
        Command::Unpack => xwin::Ops::Unpack,
        Command::Splat {
            include_debug_libs,
            include_debug_symbols,
            disable_symlinks,
            preserve_ms_arch_notation,
            copy,
            output,
        } => xwin::Ops::Splat(xwin::SplatConfig {
            include_debug_libs,
            include_debug_symbols,
            enable_symlinks: !disable_symlinks,
            preserve_ms_arch_notation,
            copy,
            output: output.unwrap_or_else(|| ctx.work_dir.join("splat")),
        }),
    };

    let pkgs = pkg_manifest.packages;

    let mp = ia::MultiProgress::with_draw_target(draw_target.into());
    let work_items: Vec<_> = pruned
        .into_iter()
        .map(|pay| {
            let prefix = match pay.kind {
                xwin::PayloadKind::CrtHeaders => "CRT.headers".to_owned(),
                xwin::PayloadKind::CrtLibs => {
                    format!(
                        "CRT.libs.{}.{}",
                        pay.target_arch.map(|ta| ta.as_str()).unwrap_or("all"),
                        pay.variant.map(|v| v.as_str()).unwrap_or("none")
                    )
                }
                xwin::PayloadKind::SdkHeaders => {
                    format!(
                        "SDK.headers.{}.{}",
                        pay.target_arch.map(|v| v.as_str()).unwrap_or("all"),
                        pay.variant.map(|v| v.as_str()).unwrap_or("none")
                    )
                }
                xwin::PayloadKind::SdkLibs => {
                    format!(
                        "SDK.libs.{}",
                        pay.target_arch.map(|ta| ta.as_str()).unwrap_or("all")
                    )
                }
                xwin::PayloadKind::SdkStoreLibs => "SDK.libs.store.all".to_owned(),
                xwin::PayloadKind::Ucrt => "SDK.ucrt.all".to_owned(),
            };

            let pb = mp.add(
                ia::ProgressBar::with_draw_target(0, draw_target.into()).with_prefix(prefix).with_style(
                    ia::ProgressStyle::default_bar()
                        .template("{spinner:.green} {prefix:.bold} [{elapsed}] {wide_bar:.green} {bytes}/{total_bytes} {msg}")
                        .unwrap()
                        .progress_chars("█▇▆▅▄▃▂▁  "),
                ),
            );
            xwin::WorkItem {
                payload: std::sync::Arc::new(pay),
                progress: pb,
            }
        })
        .collect();

    mp.set_move_cursor(true);

    let res =
        std::thread::spawn(move || ctx.execute(pkgs, work_items, arches, variants, op)).join();

    res.unwrap()
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

fn load_manifest(
    ctx: &xwin::Ctx,
    args: &Args,
    dt: xwin::util::ProgressTarget,
) -> anyhow::Result<xwin::manifest::PackageManifest> {
    let manifest_pb = ia::ProgressBar::with_draw_target(0, dt.into())
            .with_style(
            ia::ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} {prefix:.bold} [{elapsed}] {wide_bar:.green} {bytes}/{total_bytes} {msg}",
                )?
                .progress_chars("█▇▆▅▄▃▂▁  "),
        );
    manifest_pb.set_prefix("Manifest");
    manifest_pb.set_message("📥 downloading");

    let manifest = match &args.manifest {
        Some(manifest_path) => {
            let manifest_content = std::fs::read_to_string(manifest_path)
                .with_context(|| format!("failed to read path '{}'", manifest_path))?;
            serde_json::from_str(&manifest_content)
                .with_context(|| format!("failed to deserialize manifest in '{}'", manifest_path))?
        }
        None => {
            xwin::manifest::get_manifest(ctx, &args.version, &args.channel, manifest_pb.clone())?
        }
    };

    let pkg_manifest = xwin::manifest::get_package_manifest(ctx, &manifest, manifest_pb.clone())?;

    manifest_pb.finish_with_message("📥 downloaded");
    Ok(pkg_manifest)
}
