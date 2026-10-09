use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use std::{env, fs, process};

fn main() {
    let mut args = env::args().skip(1);
    let public_key = args.next().unwrap_or_else(|| usage("missing public key"));
    let artifact = args.next().unwrap_or_else(|| usage("missing artifact"));
    let signature = args.next().unwrap_or_else(|| usage("missing signature"));
    if args.next().is_some() { usage("unexpected argument"); }
    let public_key = STANDARD.decode(public_key.as_bytes()).unwrap_or_else(|error| fail(error.to_string()));
    let public_key = String::from_utf8(public_key).unwrap_or_else(|error| fail(error.to_string()));
    let public_key = PublicKey::decode(&public_key).unwrap_or_else(|error| fail(error.to_string()));
    let signature = fs::read_to_string(&signature).unwrap_or_else(|error| fail(error.to_string()));
    let signature = STANDARD.decode(signature.trim().as_bytes()).unwrap_or_else(|error| fail(error.to_string()));
    let signature = String::from_utf8(signature).unwrap_or_else(|error| fail(error.to_string()));
    let signature = Signature::decode(&signature).unwrap_or_else(|error| fail(error.to_string()));
    let bytes = fs::read(&artifact).unwrap_or_else(|error| fail(error.to_string()));
    public_key.verify(&bytes, &signature, false).unwrap_or_else(|error| fail(error.to_string()));
    println!("Verified updater signature: {artifact}");
}

fn usage(message: &str) -> ! { fail(format!("{message}; usage: verify-update-signature <pubkey> <artifact> <signature>")) }
fn fail(message: String) -> ! { eprintln!("{message}"); process::exit(1); }
