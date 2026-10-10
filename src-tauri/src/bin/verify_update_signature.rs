use std::{env, fs, process};

fn main() {
    let mut args = env::args().skip(1);
    let public_key = args.next().unwrap_or_else(|| usage("missing public key"));
    let artifact = args.next().unwrap_or_else(|| usage("missing artifact"));
    let signature = args.next().unwrap_or_else(|| usage("missing signature"));
    let version = args.next().unwrap_or_else(|| usage("missing version"));
    if args.next().is_some() {
        usage("unexpected argument");
    }
    let signature = fs::read_to_string(&signature).unwrap_or_else(|error| fail(error.to_string()));
    let bytes = fs::read(&artifact).unwrap_or_else(|error| fail(error.to_string()));
    sayweave_lib::update_signature::verify(&bytes, &public_key, &signature, &version)
        .unwrap_or_else(|error| fail(error));
    println!("Verified updater signature: {artifact}");
}

fn usage(message: &str) -> ! {
    fail(format!(
        "{message}; usage: verify-update-signature <pubkey> <artifact> <signature> <version>"
    ))
}
fn fail(message: String) -> ! {
    eprintln!("{message}");
    process::exit(1);
}
