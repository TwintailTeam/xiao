#[cfg(target_os = "linux")]
mod appimage;
mod bundle;
mod conf;
mod dev;
mod fat;
mod macos;
mod qt;
#[cfg(target_os = "linux")]
mod rpm;
#[cfg(target_os = "linux")]
mod tool;
mod util;
mod vcredist;
mod windows_portable;
mod windows_sfx;

use conf::Conf;

const USAGE: &str = "\
xiao — build tooling for xiao framework apps

usage:
  xiao dev                       run the frontend dev server and the app, restarting on change
  xiao bundle [targets…]         build in release and package (default: bundle.targets)

bundle options:
  --target <triple>              build for another target triple
                                 (universal-apple-darwin on macOS: both Mac architectures in one binary)
  --runner <cargo-sub>           build through a cargo subcommand, e.g. cargo-zigbuild
  --debug                        build the debug profile instead of release
  --no-bundle                    build the binary and stop

targets:
  deb, rpm, appimage             Linux packages, written natively
  macos-app                      a self-contained macOS .app (sign it and build the dmg in CI)
  windows-portable               a Windows folder with the Qt runtime, runs unzipped (build the installer in CI)
  windows-sfx                    one Windows .exe that unpacks that folder to %LOCALAPPDATA%\\Programs on first run
every path and name comes from xiao.conf.json next to your project root.
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("help");

    if matches!(command, "help" | "--help" | "-h") {
        print!("{USAGE}");
        return;
    }

    let conf = match Conf::load() {
        Ok(conf) => conf,
        Err(e) => fail(&e),
    };

    let result = match command {
        "dev" => dev::run(&conf),
        "bundle" | "build" => match parse_bundle_options(&args[1..]) {
            Ok(opts) => bundle::run(&conf, &opts),
            Err(e) => Err(e),
        },
        other => {
            print!("{USAGE}");
            fail(&format!("unknown command `{other}`"));
        }
    };

    if let Err(e) = result {
        fail(&e);
    }
}

fn parse_bundle_options(args: &[String]) -> Result<bundle::Options, String> {
    let mut opts = bundle::Options { targets: Vec::new(), triple: std::env::var("CARGO_BUILD_TARGET").ok(), runner: None, no_bundle: false, debug: false };

    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut take = |name: &str| -> Result<String, String> { rest.next().cloned().ok_or_else(|| format!("--{name} needs a value")) };

        match arg.as_str() {
            "--target" => opts.triple = Some(take("target")?),
            "--runner" => opts.runner = Some(take("runner")?),
            "--no-bundle" => opts.no_bundle = true,
            "--debug" => opts.debug = true,
            _ => {
                if let Some(value) = arg.strip_prefix("--target=") {
                    opts.triple = Some(value.to_owned());
                } else if let Some(value) = arg.strip_prefix("--runner=") {
                    opts.runner = Some(value.to_owned());
                } else if arg.starts_with('-') {
                    return Err(format!("unknown option `{arg}`"));
                } else {
                    opts.targets.push(arg.clone());
                }
            }
        }
    }
    Ok(opts)
}

fn fail(message: &str) -> ! {
    util::log("error", util::RED, message);
    std::process::exit(1)
}
