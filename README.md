# [bmrc](https://crates.io/crates/bmrc)

`bmrc` is a pure-Rust, dependency-free, lossless general-purpose
compression library built around **adaptive context mixing (ACM)**: a small
ensemble of order-N statistical models and a long-range LZP-style match
model feed an online-trained logistic mixer, whose output (optionally
refined by SSE/APM stages) drives a binary range coder.

```rust
use bmrc::{compress, decompress};

let data = b"hello hello hello hello world world world".repeat(10);
let compressed = compress(&data, 6);          // level 1 (fast) .. 10 (max)
let restored = decompress(&compressed).unwrap();
assert_eq!(restored, data);
```

## CLI

```text
cargo run --release --bin bmrc -- c <level 1-10> <input> <output>   # compress
cargo run --release --bin bmrc -- d <input> <output>                # decompress
cargo run --release --bin bmrc -- info <input>                      # show header
```

## How it works

For every **bit** of every byte, the predictor:

1. Looks up a probability estimate from each configured **order-k context
   model** - a hashed table of adaptive bit-probability counters, indexed by
   a hash of the previous `k` bytes plus the bits already coded within the
   current byte. Level configurations use between 2 and 13 such orders
   (`0` up to `24` bytes of context).
2. Looks up a prediction from the **match model**, an LZP-style predictor
   that follows the most recent earlier occurrence of the current context
   and predicts a literal repeat, with confidence proportional to how long
   the match has already held. This plays a role similar to an LZ77
   dictionary, but inside the probabilistic model rather than as a separate
   token stream.
3. **Mixes** all of these estimates in the logistic domain
   (`stretch`/`squash`, i.e. `ln(p/(1-p))` and its inverse) using an
   online-trained linear mixer (gradient descent on log-loss). One of
   several weight vectors is selected based on a small secondary context
   (previous byte / bit position), so the mixer can specialize.
4. Optionally refines the mixed probability through one or two **Adaptive
   Probability Map (SSE)** stages - small interpolated lookup tables that
   correct systematic bias in the mixer's output.

The resulting 12-bit probability drives a **binary range coder**
(`fpaq0`/`lpaq`-style, carry-less, 32-bit). Because the whole pipeline is
deterministic given the bit sequence seen so far, the decoder runs the
*exact same* model in lock-step and never needs to transmit any side
information beyond a 14-byte header.

```
data -> [order-0..N context models]  \
        [LZP-style match model]       >- logistic mixer -> [SSE x0-2] -> range coder -> bitstream
        [bias term]                  /
```

## Compression levels

| Level | Context orders (bytes)        | Hash table | Match model | SSE stages | Mixer contexts  |
|------:|:------------------------------|-----------:|:-----------:|:----------:|:---------------:|
| 1     | 1, 2                          | 2^16       | off         | 0          | 1               |
| 2     | 0,1,2,3                       | 2^17       | off         | 0          |        1        |
| 3     | 0,1,2,3,4                     | 2^18       | on (min 6)  | 0          |        2        |
| 4     | 0,1,2,3,4,6                   | 2^18       | on (min 5)  | 1          |        2        |
| 5     | 0,1,2,3,4,5,6                 | 2^19       | on (min 5)  | 1          |        4        |
| 6     | 0,1,2,3,4,5,6,8               | 2^20       | on (min 4)  | 1          |        4        |
| 7     | 0,1,2,3,4,5,6,8,10            | 2^21       | on (min 4)  | 2          |        8        |
| 8     | 0,1,2,3,4,5,6,8,10,12         | 2^21       | on (min 4)  | 2          |        8        |
| 9     | 0,1,2,3,4,5,6,7,8,10,12,16    | 2^22       | on (min 3)  | 2          |       16        |
| 10    | 0,1,2,3,4,5,6,7,8,10,12,16,24 | 2^22       | on (min 3)  | 2          |       16        |

Higher levels add more / longer-range context models, larger hash tables and
more SSE refinement, trading CPU time and memory for ratio. Use
[`levels::estimated_memory_bytes`] to query the approximate model memory
footprint for a given level before running.

As with any adaptive/PAQ-style compressor, **decompression takes the same
amount of work as compression** (the decoder re-runs the full model), unlike
LZ-family formats where decoding is much cheaper than encoding.

## Container format (`.bmrc`)

A BMRC stream is a 14-byte header followed by a payload:

```
Offset  Size  Field
0       4     Magic "BMR1"
4       1     Level (1-10)
5       1     Flags (bit 0 = stored verbatim, no entropy coding)
6       8     Original length, little-endian u64
14      ..    Payload (range-coder bitstream, or raw bytes if "stored")
```

If the modeled output would not be smaller than the input (e.g. for
already-compressed, encrypted, or random data), `compress` automatically
falls back to storing the data verbatim, so the output never exceeds the
input by more than the 14-byte header.

## Benchmarks (honest numbers)

These were measured in this repository with `cargo build --release`,
comparing against the system `gzip -9`, `bzip2 -9`, and `xz -9` (LZMA2).
Results **vary significantly by content type** - this is normal for
context-mixing compressors and is reported here without cherry-picking.

| Input                                      |   Size |  gzip -9 | bzip2 -9 | xz -9  | bmrc L6  |   bmrc L10 |
|--------------------------------------------|-------:|---------:|---------:|-------:|---------:|-----------:|
| BMRC research doc (Markdown, ~56 KB)       | 56,088 |   17,759 |   14,641 | 16,004 |   16,946 | **15,341** |
| Rust source (4 concatenated files, ~36 KB) | 35,809 | 10,137   |    9,371 |  9,676 |   11,135 |     9,897  |
| Highly repetitive text (20x, ~41 KB)       | 41,920 |    1,411 |    2,167 |  1,264 |    3,200 |      1,734 |

Takeaways:

- On natural-language Markdown, **level 10 beats `xz -9` by ~4%** and beats
  `gzip -9` by ~14%, using only context mixing + a match model (no
  dictionary, no BWT).
- On source code, level 10 is close to `xz -9` (within ~2%) and beats
  `gzip -9`.
- On data dominated by very long *exact* repeats, LZ77-with-huge-window
  (`xz`) still has an edge over the current match model, which is tuned for
  shorter, noisier repetitions typical of text and structured data rather
  than megabyte-scale duplication.
- All levels round-trip losslessly (see `tests/roundtrip.rs`), and
  incompressible/random data is stored verbatim rather than expanded.

**Scope vs. the BMRC research document:** the research document also
describes a BWT-based transform stage and a three-tier (global / local /
LZ77) dictionary system aimed at an aggregate +25-35% over LZMA on mixed
corpora. This crate implements the **Adaptive Context Mixer and entropy
coding stages only** - the match model substitutes for the dictionary
stages by exploiting repetition directly inside the probability model. The
BWT/dictionary stages remain future work (see "Roadmap" below); current
results should be read as "the ACM core alone, often competitive with or
better than LZMA on text-like data, occasionally behind it on
heavily-duplicated data" rather than a blanket +25% claim.

## Performance characteristics

- **Memory**: roughly `2 bytes x table_size x num_orders` for context
  tables, plus `4 bytes x match_table_size` for the match model. Level 1 uses
  a few hundred KB; level 10 uses on the order of 100-150 MB. Use
  [`levels::estimated_memory_bytes`] to compute the exact figure.
- **Speed**: single-threaded, bit-at-a-time processing. Expect roughly
  1-3 MB/s at level 1 and well under 1 MB/s at level 10 on typical hardware -
  in line with other PAQ-family compressors. This crate intentionally trades
  throughput for ratio at the higher levels; pick a lower level for
  latency-sensitive paths.
- **Determinism**: compression and decompression use the identical
  predictor, so the model never needs to be serialized - only a 14-byte
  header is added per stream.

## Integrating into another project

Add to `Cargo.toml`:

Then:

```rust
use bmrc::{compress, decompress, MIN_LEVEL, MAX_LEVEL};

fn pack(data: &[u8], user_level: u8) -> Vec<u8> {
    let level = user_level.clamp(MIN_LEVEL, MAX_LEVEL);
    compress(data, level)
}

fn unpack(data: &[u8]) -> Result<Vec<u8>, bmrc::BmrcError> {
    decompress(data)
}
```

For a container format / archiver, store the result of `compress` as an
opaque blob per entry (it is already self-describing - the level and
original length are recoverable from the header via
`bmrc::format::Header::read`).

## Roadmap

- [ ] Enhanced BWT + QLFC transform as an optional pre-pass for large blocks.
- [ ] Hierarchical dictionary system (global static dictionary + per-file
      trained dictionary) to better handle large exact repeats.
- [ ] SA-IS suffix array construction and an optimal (DP-based) LZ parser as
      an alternative front-end for very large inputs.
- [ ] rANS entropy stage as a higher-throughput alternative to the current
      binary range coder for the lower compression levels.
- [ ] SIMD-accelerated table lookups / mixing for higher throughput at high
      levels.
- [ ] Multi-threaded block-parallel compression.
