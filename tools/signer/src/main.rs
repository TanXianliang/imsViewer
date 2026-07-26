use base64::Engine;
use minisign::{SecretKeyBox, sign};
use std::fs;
use std::io::BufReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: ims-signer <key_file_b64> <file_to_sign> [password]");
        eprintln!("  key_file_b64  - base64-encoded minisign secret key file (from tauri signer generate)");
        eprintln!("  file_to_sign  - file to sign");
        eprintln!("  password      - optional key password (default: none)");
        std::process::exit(1);
    }

    let key_path = &args[1];
    let file_path = &args[2];
    let password: Option<String> = args.get(3).map(|s| s.clone()).or_else(|| Some(String::new()));

    // Read the base64-encoded key file (tauri signer format)
    let key_b64 = fs::read_to_string(key_path)?;
    let key_bytes = base64::engine::general_purpose::STANDARD.decode(key_b64.trim())?;
    let key_text = String::from_utf8(key_bytes)?;

    // Parse the SecretKeyBox from the decoded text
    let sk_box = SecretKeyBox::from_string(&key_text)?;
    let sk = sk_box.into_secret_key(password)?;

    // Open the file to sign
    let file = fs::File::open(file_path)?;
    let mut reader = BufReader::new(file);

    // Sign the file
    let signature_box = sign(None, &sk, &mut reader, None, None)?;

    // Write the signature in base64 format (matching tauri's format)
    let sig_text = signature_box.into_string();
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig_text.as_bytes());

    let sig_path = format!("{}.sig", file_path);
    fs::write(&sig_path, &sig_b64)?;

    println!("Signature written to: {}", sig_path);
    println!("Signature length: {} bytes (base64)", sig_b64.len());
    Ok(())
}
