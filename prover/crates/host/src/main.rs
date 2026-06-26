use std::{env, path::PathBuf, process};

fn main() {
    let mut args = env::args().skip(1);
    let Some(cmd) = args.next() else {
        usage();
        return;
    };

    let result = match cmd.as_str() {
        "check-vectors" => {
            let root = args
                .next()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("../bridge-vectors"));
            bridge_return_host::check_vectors(&root)
        }
        _ => {
            usage();
            Ok(())
        }
    };

    if let Err(err) = result {
        eprintln!("{err}");
        process::exit(1);
    }
}

fn usage() {
    eprintln!("usage: bridge-return-host check-vectors [../bridge-vectors]");
}
