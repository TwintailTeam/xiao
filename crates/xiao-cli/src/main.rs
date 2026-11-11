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

options:
  -c, --config <file|json>       merge a JSON file or inline JSON over xiao.conf.json (JSON Merge Patch);
                                 repeat it to merge several, in the order given

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
every path and name comes from xiao.conf.json next to your project root. xiao.linux.conf.json,
xiao.windows.conf.json and xiao.macos.conf.json next to it are merged over it for that platform.
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("help");

    if matches!(command, "help" | "--help" | "-h") {
        print!("{USAGE}");
        return;
    }

    let bundling = match command {
        "dev" => false,
        "bundle" | "build" => true,
        other => {
            print!("{USAGE}");
            fail(&format!("unknown command `{other}`"));
        }
    };

    let result = parse_options(&args[1..], bundling).and_then(|opts| {
        if bundling {
            bundle::run(&Conf::load(bundle::target_os(&opts), &opts.config)?, &opts)
        } else {
            dev::run(&Conf::load(std::env::consts::OS, &opts.config)?)
        }
    });

    if let Err(e) = result {
        fail(&e);
    }
}

fn parse_options(args: &[String], bundling: bool) -> Result<bundle::Options, String> {
    let mut opts = bundle::Options {
        targets: Vec::new(),
        triple: std::env::var("CARGO_BUILD_TARGET").ok(),
        runner: None,
        no_bundle: false,
        debug: false,
        config: Vec::new(),
    };

    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let (name, mut inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        let mut value = || inline.take().or_else(|| rest.next().cloned()).ok_or_else(|| format!("{name} needs a value"));

        match name {
            "-c" | "--config" => opts.config.push(value()?),
            "--target" if bundling => opts.triple = Some(value()?),
            "--runner" if bundling => opts.runner = Some(value()?),
            "--no-bundle" if bundling => opts.no_bundle = true,
            "--debug" if bundling => opts.debug = true,
            _ if bundling && !arg.starts_with('-') => opts.targets.push(arg.clone()),
            _ => return Err(format!("unknown option `{arg}`")),
        }
    }
    Ok(opts)
}

fn fail(message: &str) -> ! {
    util::log("error", util::RED, message);
    std::process::exit(1)
}
