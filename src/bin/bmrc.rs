//! CLI wraps `bmrc` library

use std::env;
use std::fs;
use std::process::ExitCode;
use std::time::Instant;

use bmrc::{compress, decompress, format::Header};

fn usage(program: &str) -> String {
    format!(
        "Usage:\n  \
         {program} c <level 1-10> <input> <output>   Compress\n  \
         {program} d <input> <output>                Decompress\n  \
         {program} info <input>                      Show .bmrc header info\n"
    )
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let prog = args.first().map(String::as_str).unwrap_or("bmrc");

    if args.len() < 2 {
        eprint!("{}", usage(prog));
        return ExitCode::FAILURE;
    }

    match args[1].as_str() {
        "c" if args.len() == 5 => {
            let level: u8 = match args[2].parse() {
                Ok(l) => l,
                Err(_) => {
                    eprintln!("invalid level '{}': expected a number 1-10", args[2]);
                    return ExitCode::FAILURE;
                }
            };
            let data = match fs::read(&args[3]) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("error reading '{}': {e}", args[3]);
                    return ExitCode::FAILURE;
                }
            };

            let start = Instant::now();
            let compressed = compress(&data, level);
            let elapsed = start.elapsed();

            if let Err(e) = fs::write(&args[4], &compressed) {
                eprintln!("error writing '{}': {e}", args[4]);
                return ExitCode::FAILURE;
            }

            let ratio = if data.is_empty() {
                0.0
            } else {
                compressed.len() as f64 / data.len() as f64
            };
            println!(
                "level {level}: {} -> {} bytes ({:.2}%) in {:.2?}",
                data.len(),
                compressed.len(),
                ratio * 100.0,
                elapsed
            );
            ExitCode::SUCCESS
        }
        "d" if args.len() == 4 => {
            let data = match fs::read(&args[2]) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("error reading '{}': {e}", args[2]);
                    return ExitCode::FAILURE;
                }
            };

            let start = Instant::now();
            let decompressed = match decompress(&data) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("decompression failed: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let elapsed = start.elapsed();

            if let Err(e) = fs::write(&args[3], &decompressed) {
                eprintln!("error writing '{}': {e}", args[3]);
                return ExitCode::FAILURE;
            }

            println!(
                "{} -> {} bytes in {:.2?}",
                data.len(),
                decompressed.len(),
                elapsed
            );
            ExitCode::SUCCESS
        }
        "info" if args.len() == 3 => {
            let data = match fs::read(&args[2]) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("error reading '{}': {e}", args[2]);
                    return ExitCode::FAILURE;
                }
            };
            match Header::read(&data) {
                Ok((header, payload)) => {
                    println!("level:        {}", header.level);
                    println!("flags:        0x{:02x}", header.flags);
                    println!("original len: {}", header.original_len);
                    println!("payload len:  {}", payload.len());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("invalid file: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprint!("{}", usage(prog));
            ExitCode::FAILURE
        }
    }
}
