use anyhow::Error;
use structopt::StructOpt;

fn setup_logger() -> Result<(), Error> {
    use ansi_term::Color::*;

    Ok(fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "{date} [{level}] {message}\x1B[0m",
                date = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S"),
                level = match record.level() {
                    Error => Red.paint("ERROR"),
                    Warn => Yellow.paint("WARN"),
                    Info => Green.paint("INFO"),
                    Debug => Blue.paint("DEBUG"),
                    Trace => Purple.paint("TRACE"),
                },
                message = message,
            ));
        })
        .chain(std::io::stderr())
        .apply()?)
}

#[derive(StructOpt)]
pub enum ListType {
    Workloads,
    Components,
    Packages,
}

#[derive(StructOpt)]
pub enum Command {
    Download,
    List {
        #[structopt(subcommand)]
        which: ListType,
    },
}

#[derive(StructOpt)]
pub struct Args {
    /// Doesn't display prompt to accept the license
    #[structopt(long, env = "XWIN_ACCEPT_LICENSE")]
    accept_license: bool,
    /// If set, will use a temporary directory for all files used for creating
    /// the archive and deleted upon exit, otherwise, all downloaded files
    /// are kept in the current working directory and won't be retrieved again
    #[structopt(long)]
    temp: bool,
    #[structopt(long = "work-dir")]
    work_dir: Option<std::path::PathBuf>,
    /// The version to retrieve, can either be a major version of 15 or 16, or
    /// a "<major>.<minor>" version.
    #[structopt(long, default_value = "16")]
    version: String,
    /// The product channel to use.
    #[structopt(long, default_value = "release")]
    channel: String,
    #[structopt(subcommand)]
    cmd: Command,
}

#[tokio::main(threaded_scheduler)]
async fn main() -> Result<(), Error> {
    setup_logger()?;

    let args = Args::from_args();

    let ctx = if args.temp {
        xwin::Ctx::with_temp()?
    } else {
        xwin::Ctx::with_dir(args.work_dir.unwrap_or_else(|| {
            std::env::current_dir().expect("failed to get current working dir")
        }))?
    };

    xwin::execute(ctx).await;

    xwin::Ok(())
}
