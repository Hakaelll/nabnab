use std::{env, fs::OpenOptions, io::Write, path::PathBuf};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let root = PathBuf::from(env::var_os("NABNAB_TEST_DATA_DIR").unwrap());
    let has = |flag: &str| args.iter().any(|arg| arg == flag);
    if has("--help") {
        println!("--share <path or URL>\n--share-text <text>\n--id-name-only");
    } else if has("--my-id") {
        println!("test-desktop-id");
    } else if has("--list-available") {
        if root.join("unreachable").is_file() {
            eprintln!("No devices found");
        } else if has("--id-name-only") {
            println!("paired-device-123 Teléfono de prueba ñ");
        } else {
            println!("paired-device-123");
        }
    } else if args
        .iter()
        .any(|arg| arg.starts_with("--share=") || arg.starts_with("--share-text="))
    {
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("cli-arguments.jsonl"))
            .unwrap();
        writeln!(log, "{args:?}").unwrap();
        if root.join("cli-warning").is_file() {
            eprintln!("D-Bus share interface unavailable");
        }
        if let Some(text) = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--share-text="))
        {
            println!("Shared text: {text}");
        } else if let Some(url) = args.iter().find_map(|arg| arg.strip_prefix("--share=")) {
            println!("Shared {url}");
            if !root.join("cli-first-file-only").is_file() {
                for url in args.iter().skip_while(|arg| *arg != "--").skip(1) {
                    println!("Shared {url}");
                }
            }
        }
    } else {
        std::process::exit(1);
    }
}
