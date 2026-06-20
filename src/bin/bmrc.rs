//! CLI wraps `bmrc` library

use std::env;
use std::fs;
use std::process::ExitCode;
use std::time::Instant;

use bmrc::{compress, compress_bwt, compress_parallel, decompress, decompress_parallel, format::Header};

fn usage(program: &str) -> String {
    format!(
        "Usage:\n  \
         {program} c  <level 1-10> <input> <output>             Compress\n  \
         {program} d  <input> <output>                          Decompress\n  \
         {program} cb <level 1-10> <input> <output>             Compress with BWT pre-pass\n  \
         {program} cp <level 1-10> <block_kb> <input> <output>  Compress in parallel\n  \
         {program} dp <input> <output>                          Decompress parallel stream\n  \
         {program} info <input>                                 Show .bmrc header info\n"
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
        "cb" if args.len() == 5 => {
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
            let compressed = compress_bwt(&data, level);
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
                "level {level} (BWT): {} -> {} bytes ({:.2}%) in {:.2?}",
                data.len(),
                compressed.len(),
                ratio * 100.0,
                elapsed
            );
            ExitCode::SUCCESS
        }
        "cp" if args.len() == 6 => {
            let level: u8 = match args[2].parse() {
                Ok(l) => l,
                Err(_) => {
                    eprintln!("invalid level '{}': expected a number 1-10", args[2]);
                    return ExitCode::FAILURE;
                }
            };
            let block_kb: usize = match args[3].parse() {
                Ok(k) if k > 0 => k,
                _ => {
                    eprintln!("invalid block size '{}': expected a positive number (KB)", args[3]);
                    return ExitCode::FAILURE;
                }
            };
            let data = match fs::read(&args[4]) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("error reading '{}': {e}", args[4]);
                    return ExitCode::FAILURE;
                }
            };

            let block_size = block_kb * 1024;
            let n_blocks = if data.is_empty() { 1 } else { data.len().div_ceil(block_size) };

            let start = Instant::now();
            let compressed = compress_parallel(&data, level, Some(block_size));
            let elapsed = start.elapsed();

            if let Err(e) = fs::write(&args[5], &compressed) {
                eprintln!("error writing '{}': {e}", args[5]);
                return ExitCode::FAILURE;
            }

            let ratio = if data.is_empty() {
                0.0
            } else {
                compressed.len() as f64 / data.len() as f64
            };
            println!(
                "level {level}, {n_blocks} blocks x {block_kb} KB: {} -> {} bytes ({:.2}%) in {:.2?}",
                data.len(),
                compressed.len(),
                ratio * 100.0,
                elapsed
            );
            ExitCode::SUCCESS
        }
        "dp" if args.len() == 4 => {
            let data = match fs::read(&args[2]) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("error reading '{}': {e}", args[2]);
                    return ExitCode::FAILURE;
                }
            };

            let start = Instant::now();
            let decompressed = match decompress_parallel(&data) {
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
