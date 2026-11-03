# chain-lens

`chain-lens` is a small Rust CLI and library for checking whether multiple
sampling chains have mixed. It reads scalar draws from JSONL and reports:

- rank-normalized, folded split-R-hat;
- bulk effective sample size (ESS);
- tail ESS at the 5th and 95th percentiles;
- deterministic, machine-readable threshold findings.

It rejects duplicate draws, non-finite values, unequal chain lengths, schema
drift, fewer than two chains, and chains with fewer than four draws. Odd-length
chains are supported; one middle draw per chain is excluded from split-chain
diagnostics and the report says how many draws were used.

## Install

```console
cargo install --git https://github.com/nicholas-ruest/chain-lens
```

Or build from a checkout with stable Rust 1.85 or newer:

```console
cargo build --release
```

## Input

Each line identifies a chain, a draw number, and one or more scalar values:

```json
{"chain":"north","draw":0,"values":{"mu":-0.20,"sigma":1.02}}
{"chain":"south","draw":0,"values":{"mu":0.10,"sigma":0.98}}
```

Every draw must have the same variables. Chain names and draw numbers may be in
any input order; `chain-lens` sorts them before analysis.

## Use

Run the included short example with relaxed demonstration thresholds:

```console
cargo run -- examples/draws.jsonl --max-rhat 1.2 --min-ess 4 --pretty
```

Use production-style defaults (`R-hat <= 1.01`, bulk/tail `ESS >= 100`) on a
larger result:

```console
chain-lens posterior.jsonl > diagnostics.json
```

Standard input works too:

```console
cat posterior.jsonl | chain-lens --min-ess 400 --pretty
```

Exit codes are suitable for pipelines:

- `0`: every variable passes the supplied thresholds;
- `2`: valid analysis, but at least one variable fails a threshold;
- `1`: malformed input, invalid options, or an I/O failure.

## Output

Reports include summary statistics, every diagnostic, threshold-specific issue
messages, and an aggregate `converged` boolean:

```json
{
  "schema_version": 1,
  "input": {"chains": 4, "draws_per_chain": 1000, "draws_used_per_chain": 1000, "variables": 2},
  "thresholds": {"max_rhat": 1.01, "min_ess": 100.0},
  "variables": {
    "mu": {"mean": 0.01, "standard_deviation": 0.98, "median": 0.0, "rhat": 1.001, "bulk_ess": 3650.2, "tail_ess": 3411.7, "passed": true, "issues": []}
  },
  "converged": true
}
```

These diagnostics detect common non-convergence patterns; they do not prove
that a target distribution is correct, that a sampler implementation is
unbiased, or that every observable has been explored.

## Development

```console
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

The implementation uses no `unsafe` code. Continuous integration repeats all
three checks and verifies the declared Rust 1.85 minimum version.

## License

MIT
