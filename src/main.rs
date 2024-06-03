#[cfg(all(target_env = "musl", target_arch = "x86_64"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use anyhow::{Context as _, Error};
use camino::Utf8PathBuf as PathBuf;
use clap::builder::{PossibleValuesParser, TypedValueParser as _};
use clap::{Parser, Subcommand};
use indicatif as ia;
use std::time::Duration;
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

#[derive(Parser)]
pub struct SplatOptions {
    /// The MSVCRT includes (non-redistributable) debug versions of the
    /// various libs that are generally uninteresting to keep for most usage
    #[arg(long)]
    include_debug_libs: bool,
    /// The MSVCRT includes PDB (debug symbols) files for several of the
    /// libraries that are generally uninteresting to keep for most usage
    #[arg(long)]
    include_debug_symbols: bool,
    /// By default, symlinks are added to both the CRT and `WindowsSDK` to
    /// address casing issues in general usage.
    ///
    /// For example, if you are compiling C/C++ code that does
    /// `#include <windows.h>`, it will break on a case-sensitive file system,
    /// as the actual path in the `WindowsSDK` is `Windows.h`. This also applies
    /// even if the C/C++ you are compiling uses correct casing for all CRT/SDK
    /// includes, as the internal headers also use incorrect casing in most cases.
    #[arg(long)]
    disable_symlinks: bool,
    /// By default, we convert the MS specific `x64`, `arm`, and `arm64`
    /// target architectures to the more canonical `x86_64`, `aarch`, and
    /// `aarch64` of LLVM etc when creating directories/names.
    ///
    /// Passing this flag will preserve the MS names for those targets.
    #[arg(long)]
    preserve_ms_arch_notation: bool,
    /// Use the /winsysroot layout, so that clang-cl's /winsysroot flag can be
    /// used with the output, rather than needing both -vctoolsdir and
    /// -winsdkdir. You will likely also want to use --preserve-ms-arch-notation
    /// and --disable-symlinks for use with clang-cl on Windows.
    #[arg(long)]
    use_winsysroot_style: bool,
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
        #[command(flatten)]
        options: SplatOptions,
        /// The root output directory. Defaults to `./.xwin-cache/splat` if not
        /// specified.
        #[arg(long)]
        output: Option<PathBuf>,
        /// If specified, a toml file that can be used to create additional symlinks
        /// or skip files entirely
        #[arg(long)]
        map: Option<PathBuf>,
        /// Copies files from the unpack directory to the splat directory instead
        /// of moving them, which preserves the original unpack directories but
        /// increases overall time and disk usage
        #[arg(long)]
        copy: bool,
        // Splits the CRT and SDK into architecture and variant specific
        // directories. The shared headers in the CRT and SDK are duplicated
        // for each output so that each combination is self-contained.
        // #[arg(long)]
        // isolated: bool,
    },
    /// Runs the specified build command, detecting all of the headers and libraries
    /// used by the build, and generating a file that can be used to filter future
    /// splat operations, and optionally move only the user files to a new directory
    ///
    /// This command is only intended to work with cargo builds
    ///
    /// This command requires that `strace`, `clang-cl` and `lld-link` are installed
    /// and _probably_ only works on Linux.
    Minimize {
        #[command(flatten)]
        options: SplatOptions,
        /// The path of the filter file that is generated. Defaults to ./.xwin-cache/xwin-map.toml
        #[arg(long)]
        map: Option<PathBuf>,
        /// The root splat output directory. Defaults to `./.xwin-cache/splat` if not
        /// specified.
        #[arg(long)]
        output: Option<PathBuf>,
        /// The root output directory for the minimized set of files discovered during
        /// the build. If not specified only the map file is written in addition
        /// to the splat.
        #[arg(long)]
        minimize_output: Option<PathBuf>,
        /// Copies files from the splat directory rather than moving them. Only
        /// used if --output is specified.
        #[arg(long)]
        copy: bool,
        /// The cargo build triple to compile for. Defaults to `x86_64-pc-windows-msvc`
        /// if not specified
        #[arg(long)]
        target: Option<String>,
        /// The path of the manifest to compile. Defaults to Cargo.toml if not specified
        #[arg(long)]
        manifest_path: Option<PathBuf>,
        /// If supplied, the strace output is persisted to disk rather than being
        /// deleted once the compilation has finished
        #[arg(long)]
        preserve_strace: bool,
    },
}

const ARCHES: &[&str] = &["x86", "x86_64", "aarch", "aarch64"];
const VARIANTS: &[&str] = &["desktop", "onecore", /*"store",*/ "spectre"];
const LOG_LEVELS: &[&str] = &["off", "error", "warn", "info", "debug", "trace"];

fn parse_level(s: &str) -> Result<LevelFilter, Error> {
    s.parse::<LevelFilter>()
        .map_err(|_e| anyhow::anyhow!("failed to parse level '{s}'"))
}

#[allow(clippy::indexing_slicing)]
fn parse_duration(src: &str) -> anyhow::Result<Duration> {
    let suffix_pos = src.find(char::is_alphabetic).unwrap_or(src.len());

    let num: u64 = src[..suffix_pos].parse()?;
    let suffix = if suffix_pos == src.len() {
        "s"
    } else {
        &src[suffix_pos..]
    };

    let duration = match suffix {
        "ms" => Duration::from_millis(num),
        "s" | "S" => Duration::from_secs(num),
        "m" | "M" => Duration::from_secs(num * 60),
        "h" | "H" => Duration::from_secs(num * 60 * 60),
        "d" | "D" => Duration::from_secs(num * 60 * 60 * 24),
        s => anyhow::bail!("unknown duration suffix '{s}'"),
    };

    Ok(duration)
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Doesn't display the prompt to accept the license
    #[arg(long, env = "XWIN_ACCEPT_LICENSE")]
    accept_license: bool,
    /// The log level for messages, only log messages at or above the level will be emitted.
    #[arg(
        short = 'L',
        long = "log-level",
        default_value = "info",
        value_parser = PossibleValuesParser::new(LOG_LEVELS).map(|l| parse_level(&l).unwrap()),
    )]
    level: LevelFilter,
    /// Output log messages as json
    #[arg(long)]
    json: bool,
    /// If set, will use a temporary directory for all files used for creating
    /// the archive and deleted upon exit, otherwise, all downloaded files
    /// are kept in the `--cache-dir` and won't be retrieved again
    #[arg(long)]
    temp: bool,
    /// Specifies the cache directory used to persist downloaded items to disk.
    /// Defaults to `./.xwin-cache` if not specified.
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    /// Specifies a VS manifest to use from a file, rather than downloading it
    /// from the Microsoft site.
    #[arg(long, conflicts_with_all = &["manifest_version", "channel"])]
    manifest: Option<PathBuf>,
    /// The manifest version to retrieve
    #[arg(long, default_value = "17")]
    manifest_version: String,
    /// The product channel to use.
    #[arg(long, default_value = "release")]
    channel: String,
    /// If specified, this is the version of the SDK that the user wishes to use
    /// instead of defaulting to the latest SDK available in the the manifest
    #[arg(long)]
    sdk_version: Option<String>,
    /// If specified, this is the version of the MSVCRT that the user wishes to use
    /// instead of defaulting to the latest MSVCRT available in the the manifest
    #[arg(long)]
    crt_version: Option<String>,
    /// Whether to include the Active Template Library (ATL) in the installation
    #[arg(long)]
    include_atl: bool,
    /// Specifies a timeout for how long a single download is allowed to take.
    #[arg(short, long, value_parser = parse_duration, default_value = "60s")]
    timeout: Duration,
    /// An HTTPS proxy to use
    #[arg(long, env = "HTTPS_PROXY")]
    https_proxy: Option<String>,
    /// The architectures to include
    #[arg(
        long,
        value_parser = PossibleValuesParser::new(ARCHES).map(|s| s.parse::<xwin::Arch>().unwrap()),
        value_delimiter = ',',
        default_values_t = vec![xwin::Arch::X86_64],
    )]
    arch: Vec<xwin::Arch>,
    /// The variants to include
    #[arg(
        long,
        value_parser = PossibleValuesParser::new(VARIANTS).map(|s| s.parse::<xwin::Variant>().unwrap()),
        value_delimiter = ',',
        default_values_t = vec![xwin::Variant::Desktop],
    )]
    variant: Vec<xwin::Variant>,
    #[command(subcommand)]
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
            other => anyhow::bail!("unknown response to license request {other}"),
        }
    }

    let cwd = PathBuf::from_path_buf(std::env::current_dir().context("unable to retrieve cwd")?)
        .map_err(|pb| anyhow::anyhow!("cwd {} is not a valid utf-8 path", pb.display()))?;

    let draw_target = xwin::util::ProgressTarget::Stdout;

    let client = {
        let mut builder = ureq::AgentBuilder::new().timeout_read(args.timeout);

        if let Some(proxy) = args.https_proxy {
            let proxy = ureq::Proxy::new(proxy).context("failed to parse https proxy address")?;
            builder = builder.proxy(proxy);
        }

        builder.build()
    };

    let ctx = if args.temp {
        xwin::Ctx::with_temp(draw_target, client)?
    } else {
        let cache_dir = match &args.cache_dir {
            Some(cd) => cd.clone(),
            None => cwd.join(".xwin-cache"),
        };
        xwin::Ctx::with_dir(cache_dir, draw_target, client)?
    };

    let ctx = std::sync::Arc::new(ctx);

    let pkg_manifest = load_manifest(
        &ctx,
        args.manifest.as_ref(),
        &args.manifest_version,
        &args.channel,
        draw_target,
    )?;

    let arches = args.arch.into_iter().fold(0, |acc, arch| acc | arch as u32);
    let variants = args
        .variant
        .into_iter()
        .fold(0, |acc, var| acc | var as u32);

    let pruned = xwin::prune_pkg_list(
        &pkg_manifest,
        arches,
        variants,
        args.include_atl,
        args.sdk_version,
        args.crt_version,
    )?;

    let op = match args.cmd {
        Command::List => {
            print_packages(&pruned.payloads);
            return Ok(());
        }
        Command::Download => xwin::Ops::Download,
        Command::Unpack => xwin::Ops::Unpack,
        Command::Splat {
            options,
            copy,
            map,
            output,
        } => xwin::Ops::Splat(xwin::SplatConfig {
            include_debug_libs: options.include_debug_libs,
            include_debug_symbols: options.include_debug_symbols,
            enable_symlinks: !options.disable_symlinks,
            preserve_ms_arch_notation: options.preserve_ms_arch_notation,
            use_winsysroot_style: options.use_winsysroot_style,
            copy,
            map,
            output: output.unwrap_or_else(|| ctx.work_dir.join("splat")),
        }),
        Command::Minimize {
            map,
            output,
            copy,
            minimize_output,
            options,
            target,
            manifest_path,
            preserve_strace,
        } => xwin::Ops::Minimize(xwin::MinimizeConfig {
            include_debug_libs: options.include_debug_libs,
            include_debug_symbols: options.include_debug_symbols,
            enable_symlinks: !options.disable_symlinks,
            preserve_ms_arch_notation: options.preserve_ms_arch_notation,
            use_winsysroot_style: options.use_winsysroot_style,
            splat_output: output.unwrap_or_else(|| ctx.work_dir.join("splat")),
            copy,
            minimize_output,
            map: map.unwrap_or_else(|| ctx.work_dir.join("xwin-map.toml")),
            target: target.unwrap_or("x86_64-pc-windows-msvc".to_owned()),
            manifest_path: manifest_path.unwrap_or("Cargo.toml".into()),
            preserve_strace,
        }),
    };

    let pkgs = pkg_manifest.packages;

    let mp = ia::MultiProgress::with_draw_target(draw_target.into());
    let work_items: Vec<_> = pruned
        .payloads
        .into_iter()
        .map(|pay| {
            use xwin::PayloadKind;

            let prefix = match pay.kind {
                PayloadKind::CrtHeaders => "CRT.headers".to_owned(),
                PayloadKind::AtlHeaders => "ATL.headers".to_owned(),
                PayloadKind::CrtLibs => {
                    format!(
                        "CRT.libs.{}.{}",
                        pay.target_arch.map_or("all", |ta| ta.as_str()),
                        pay.variant.map_or("none", |v| v.as_str())
                    )
                }
                PayloadKind::AtlLibs => {
                    format!(
                        "ATL.libs.{}",
                        pay.target_arch.map_or("all", |ta| ta.as_str()),
                    )
                }
                PayloadKind::SdkHeaders => {
                    format!(
                        "SDK.headers.{}.{}",
                        pay.target_arch.map_or("all", |v| v.as_str()),
                        pay.variant.map_or("none", |v| v.as_str())
                    )
                }
                PayloadKind::SdkLibs => {
                    format!(
                        "SDK.libs.{}",
                        pay.target_arch.map_or("all", |ta| ta.as_str())
                    )
                }
                PayloadKind::SdkStoreLibs => "SDK.libs.store.all".to_owned(),
                PayloadKind::Ucrt => "SDK.ucrt.all".to_owned(),
            };

            let pb = mp.add(
                ia::ProgressBar::with_draw_target(Some(0), draw_target.into()).with_prefix(prefix).with_style(
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

    let res = std::thread::spawn(move || {
        ctx.execute(
            pkgs,
            work_items,
            pruned.crt_version,
            pruned.sdk_version,
            arches,
            variants,
            op,
        )
    })
    .join();

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
    manifest: Option<&PathBuf>,
    manifest_version: &str,
    channel: &str,
    dt: xwin::util::ProgressTarget,
) -> anyhow::Result<xwin::manifest::PackageManifest> {
    let manifest_pb = ia::ProgressBar::with_draw_target(Some(0), dt.into())
            .with_style(
            ia::ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} {prefix:.bold} [{elapsed}] {wide_bar:.green} {bytes}/{total_bytes} {msg}",
                )?
                .progress_chars("█▇▆▅▄▃▂▁  "),
        );
    manifest_pb.set_prefix("Manifest");
    manifest_pb.set_message("📥 downloading");

    let manifest = match manifest {
        Some(manifest_path) => {
            let manifest_content = std::fs::read_to_string(manifest_path)
                .with_context(|| format!("failed to read path '{}'", manifest_path))?;
            serde_json::from_str(&manifest_content)
                .with_context(|| format!("failed to deserialize manifest in '{}'", manifest_path))?
        }
        None => xwin::manifest::get_manifest(ctx, manifest_version, channel, manifest_pb.clone())?,
    };

    let pkg_manifest = xwin::manifest::get_package_manifest(ctx, &manifest, manifest_pb.clone())?;

    manifest_pb.finish_with_message("📥 downloaded");
    Ok(pkg_manifest)
}

#[cfg(test)]
mod test {
    #[test]
    fn cli_help() {
        use clap::CommandFactory;

        let snapshot_path = format!("{}/tests/snapshots", env!("CARGO_MANIFEST_DIR"));

        insta::with_settings!({
            prepend_module_to_snapshot => false,
            snapshot_path => snapshot_path,
        }, {
            // the tests here will force maps to sort
            snapshot_test_cli_command(super::Args::command().name(env!("CARGO_PKG_NAME")), "xwin".to_owned(), &SnapshotTestDesc {
                manifest_path: env!("CARGO_MANIFEST_DIR"),
                module_path: module_path!(),
                file: file!(),
                line: line!(),
            });
        });
    }

    use clap::{ColorChoice, Command};

    pub struct SnapshotTestDesc {
        pub manifest_path: &'static str,
        pub module_path: &'static str,
        pub file: &'static str,
        pub line: u32,
    }

    fn snapshot_test_cli_command(app: Command, cmd_name: String, desc: &SnapshotTestDesc) {
        let mut app = app
            // we do not want ASCII colors in our snapshot test output
            .color(ColorChoice::Never)
            // override versions to not have to update test when changing versions
            .version("0.0.0")
            .long_version("0.0.0")
            .term_width(80);

        // don't show current env vars as that will make snapshot test output diff depending on environment run in
        let arg_names = app
            .get_arguments()
            .map(|a| a.get_id().clone())
            .filter(|a| *a != "version" && *a != "help")
            .collect::<Vec<_>>();
        for arg_name in arg_names {
            app = app.mut_arg(arg_name, |arg| arg.hide_env_values(true));
        }

        // get the long help text for the command
        let mut buffer = Vec::new();
        app.write_long_help(&mut buffer).unwrap();
        let help_text = std::str::from_utf8(&buffer).unwrap();

        // use internal `insta` function instead of the macro so we can pass in the
        // right module information from the crate and to gather up the errors instead of panicking directly on failures
        insta::_macro_support::assert_snapshot(
            cmd_name.clone().into(),
            help_text,
            desc.manifest_path,
            "cli-cmd",
            desc.module_path,
            desc.file,
            desc.line,
            "help_text",
        )
        .unwrap();

        // recursively test all subcommands
        for app in app.get_subcommands() {
            if app.get_name() == "help" {
                continue;
            }

            snapshot_test_cli_command(
                app.clone(),
                format!("{}-{}", cmd_name, app.get_name()),
                desc,
            );
        }
    }
}
