//! MCMC convergence diagnostics for line-delimited JSON draws.
//!
//! `chain-lens` computes the rank-normalized, folded split-R-hat diagnostic and
//! bulk/tail effective sample sizes described by Vehtari et al. The library is
//! intentionally strict about its input so incomplete chains do not quietly
//! produce reassuring reports.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use statrs::distribution::{ContinuousCDF, Normal};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;
use thiserror::Error;

const SCHEMA_VERSION: u8 = 1;

/// One JSONL observation.
#[derive(Debug, Clone, Deserialize)]
pub struct DrawRecord {
    /// Stable chain identifier.
    pub chain: String,
    /// Draw number, unique within the chain.
    pub draw: u64,
    /// Named scalar variables observed at this draw.
    pub values: BTreeMap<String, f64>,
}

/// Admission thresholds applied to every variable.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Thresholds {
    /// Largest acceptable rank-normalized split-R-hat.
    pub max_rhat: f64,
    /// Smallest acceptable bulk and tail effective sample size.
    pub min_ess: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            max_rhat: 1.01,
            min_ess: 100.0,
        }
    }
}

/// Complete serializable analysis report.
#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticsReport {
    pub schema_version: u8,
    pub input: InputSummary,
    pub thresholds: Thresholds,
    pub variables: BTreeMap<String, VariableDiagnostics>,
    pub converged: bool,
}

/// Shape of the validated input.
#[derive(Debug, Clone, Serialize)]
pub struct InputSummary {
    pub chains: usize,
    pub draws_per_chain: usize,
    pub draws_used_per_chain: usize,
    pub variables: usize,
}

/// Diagnostics and threshold findings for one scalar variable.
#[derive(Debug, Clone, Serialize)]
pub struct VariableDiagnostics {
    pub mean: f64,
    pub standard_deviation: f64,
    pub median: f64,
    pub rhat: f64,
    pub bulk_ess: f64,
    pub tail_ess: f64,
    pub passed: bool,
    pub issues: Vec<String>,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("line {line}: invalid JSON: {source}")]
    Json {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("line {line}: chain must not be empty")]
    EmptyChain { line: usize },
    #[error("line {line}: values must not be empty")]
    EmptyValues { line: usize },
    #[error("line {line}: variable name must not be empty")]
    EmptyVariable { line: usize },
    #[error("line {line}: variable {variable:?} is not finite")]
    NonFinite { line: usize, variable: String },
    #[error("duplicate draw {draw} in chain {chain:?}")]
    DuplicateDraw { chain: String, draw: u64 },
    #[error("input contains no draws")]
    NoDraws,
    #[error("at least two chains are required; found {0}")]
    TooFewChains(usize),
    #[error("each chain needs at least four draws; chain {chain:?} has {draws}")]
    TooFewDraws { chain: String, draws: usize },
    #[error(
        "all chains must have equal length; expected {expected}, chain {chain:?} has {actual}"
    )]
    UnequalChainLength {
        expected: usize,
        chain: String,
        actual: usize,
    },
    #[error("all draws must contain the same variables; chain {chain:?} draw {draw} differs")]
    VariableMismatch { chain: String, draw: u64 },
    #[error("max_rhat must be finite and at least 1.0")]
    InvalidRhatThreshold,
    #[error("min_ess must be finite and greater than 0")]
    InvalidEssThreshold,
}

type DrawValues = BTreeMap<String, f64>;
type Chain = BTreeMap<u64, DrawValues>;

/// Parse, validate, and analyze JSONL draws from `reader`.
pub fn analyze_reader<R: BufRead>(
    reader: R,
    thresholds: Thresholds,
) -> Result<DiagnosticsReport, Error> {
    validate_thresholds(thresholds)?;
    let mut chains: BTreeMap<String, Chain> = BTreeMap::new();

    for (index, line_result) in reader.lines().enumerate() {
        let line_number = index + 1;
        let line = line_result.map_err(|source| Error::Json {
            line: line_number,
            source: serde_json::Error::io(source),
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let record: DrawRecord = serde_json::from_str(&line).map_err(|source| Error::Json {
            line: line_number,
            source,
        })?;
        validate_record(&record, line_number)?;
        let chain = chains.entry(record.chain.clone()).or_default();
        if chain.insert(record.draw, record.values).is_some() {
            return Err(Error::DuplicateDraw {
                chain: record.chain,
                draw: record.draw,
            });
        }
    }

    analyze_chains(chains, thresholds)
}

fn validate_thresholds(thresholds: Thresholds) -> Result<(), Error> {
    if !thresholds.max_rhat.is_finite() || thresholds.max_rhat < 1.0 {
        return Err(Error::InvalidRhatThreshold);
    }
    if !thresholds.min_ess.is_finite() || thresholds.min_ess <= 0.0 {
        return Err(Error::InvalidEssThreshold);
    }
    Ok(())
}

fn validate_record(record: &DrawRecord, line: usize) -> Result<(), Error> {
    if record.chain.trim().is_empty() {
        return Err(Error::EmptyChain { line });
    }
    if record.values.is_empty() {
        return Err(Error::EmptyValues { line });
    }
    for (name, value) in &record.values {
        if name.trim().is_empty() {
            return Err(Error::EmptyVariable { line });
        }
        if !value.is_finite() {
            return Err(Error::NonFinite {
                line,
                variable: name.clone(),
            });
        }
    }
    Ok(())
}

fn analyze_chains(
    chains: BTreeMap<String, Chain>,
    thresholds: Thresholds,
) -> Result<DiagnosticsReport, Error> {
    if chains.is_empty() {
        return Err(Error::NoDraws);
    }
    if chains.len() < 2 {
        return Err(Error::TooFewChains(chains.len()));
    }

    let expected_draws = chains.values().next().map(BTreeMap::len).unwrap_or(0);
    let expected_variables: BTreeSet<String> = chains
        .values()
        .next()
        .and_then(|chain| chain.values().next())
        .map(|values| values.keys().cloned().collect())
        .unwrap_or_default();

    for (chain_name, chain) in &chains {
        if chain.len() < 4 {
            return Err(Error::TooFewDraws {
                chain: chain_name.clone(),
                draws: chain.len(),
            });
        }
        if chain.len() != expected_draws {
            return Err(Error::UnequalChainLength {
                expected: expected_draws,
                chain: chain_name.clone(),
                actual: chain.len(),
            });
        }
        for (draw, values) in chain {
            let variables: BTreeSet<String> = values.keys().cloned().collect();
            if variables != expected_variables {
                return Err(Error::VariableMismatch {
                    chain: chain_name.clone(),
                    draw: *draw,
                });
            }
        }
    }

    let draws_used = expected_draws - (expected_draws % 2);
    let mut variables = BTreeMap::new();
    for variable in &expected_variables {
        let raw_chains: Vec<Vec<f64>> = chains
            .values()
            .map(|chain| {
                chain
                    .values()
                    .map(|values| values[variable])
                    .collect::<Vec<_>>()
            })
            .collect();
        variables.insert(variable.clone(), diagnose_variable(&raw_chains, thresholds));
    }
    let converged = variables.values().all(|diagnostic| diagnostic.passed);

    Ok(DiagnosticsReport {
        schema_version: SCHEMA_VERSION,
        input: InputSummary {
            chains: chains.len(),
            draws_per_chain: expected_draws,
            draws_used_per_chain: draws_used,
            variables: expected_variables.len(),
        },
        thresholds,
        variables,
        converged,
    })
}

fn diagnose_variable(chains: &[Vec<f64>], thresholds: Thresholds) -> VariableDiagnostics {
    let all: Vec<f64> = chains.iter().flatten().copied().collect();
    let mean = arithmetic_mean(&all);
    let standard_deviation = sample_variance(&all).sqrt();
    let median = quantile(&all, 0.5);
    let split = split_chains(chains);

    let normalized = rank_normalize(&split);
    let rank_rhat = split_rhat(&normalized);
    let folded: Vec<Vec<f64>> = split
        .iter()
        .map(|chain| chain.iter().map(|value| (value - median).abs()).collect())
        .collect();
    let folded_rhat = split_rhat(&rank_normalize(&folded));
    let rhat = rank_rhat.max(folded_rhat);
    let bulk_ess = effective_sample_size(&normalized);

    let lower = quantile(&all, 0.05);
    let upper = quantile(&all, 0.95);
    let lower_indicator = map_chains(&split, |value| f64::from(value <= lower));
    let upper_indicator = map_chains(&split, |value| f64::from(value >= upper));
    let tail_ess =
        effective_sample_size(&lower_indicator).min(effective_sample_size(&upper_indicator));

    let mut issues = Vec::new();
    if rhat > thresholds.max_rhat {
        issues.push(format!(
            "rhat {rhat:.4} exceeds maximum {:.4}",
            thresholds.max_rhat
        ));
    }
    if bulk_ess < thresholds.min_ess {
        issues.push(format!(
            "bulk_ess {bulk_ess:.1} is below minimum {:.1}",
            thresholds.min_ess
        ));
    }
    if tail_ess < thresholds.min_ess {
        issues.push(format!(
            "tail_ess {tail_ess:.1} is below minimum {:.1}",
            thresholds.min_ess
        ));
    }

    VariableDiagnostics {
        mean,
        standard_deviation,
        median,
        rhat,
        bulk_ess,
        tail_ess,
        passed: issues.is_empty(),
        issues,
    }
}

fn split_chains(chains: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let draws = chains[0].len();
    let half = draws / 2;
    let mut split = Vec::with_capacity(chains.len() * 2);
    for chain in chains {
        split.push(chain[..half].to_vec());
        split.push(chain[draws - half..].to_vec());
    }
    split
}

fn map_chains<F>(chains: &[Vec<f64>], mut transform: F) -> Vec<Vec<f64>>
where
    F: FnMut(f64) -> f64,
{
    chains
        .iter()
        .map(|chain| chain.iter().copied().map(&mut transform).collect())
        .collect()
}

fn rank_normalize(chains: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let width = chains[0].len();
    let flat: Vec<f64> = chains.iter().flatten().copied().collect();
    let ranks = midranks(&flat);
    let total = flat.len() as f64;
    let normal = Normal::standard();
    let normalized: Vec<f64> = ranks
        .into_iter()
        .map(|rank| normal.inverse_cdf((rank - 0.375) / (total + 0.25)))
        .collect();
    normalized.chunks(width).map(<[f64]>::to_vec).collect()
}

fn midranks(values: &[f64]) -> Vec<f64> {
    let mut indexed: Vec<(usize, f64)> = values.iter().copied().enumerate().collect();
    indexed.sort_by(|left, right| left.1.total_cmp(&right.1));
    let mut ranks = vec![0.0; values.len()];
    let mut start = 0;
    while start < indexed.len() {
        let mut end = start + 1;
        while end < indexed.len() && indexed[end].1.total_cmp(&indexed[start].1) == Ordering::Equal
        {
            end += 1;
        }
        let rank = ((start + 1 + end) as f64) / 2.0;
        for &(original, _) in &indexed[start..end] {
            ranks[original] = rank;
        }
        start = end;
    }
    ranks
}

fn split_rhat(chains: &[Vec<f64>]) -> f64 {
    let chain_count = chains.len() as f64;
    let draws = chains[0].len() as f64;
    let means: Vec<f64> = chains.iter().map(|chain| arithmetic_mean(chain)).collect();
    let within = chains
        .iter()
        .map(|chain| sample_variance(chain))
        .sum::<f64>()
        / chain_count;
    let between = draws * sample_variance(&means);
    let variance = ((draws - 1.0) / draws) * within + between / draws;
    if within <= f64::EPSILON {
        if variance <= f64::EPSILON {
            1.0
        } else {
            f64::MAX
        }
    } else {
        (variance / within).sqrt()
    }
}

fn effective_sample_size(chains: &[Vec<f64>]) -> f64 {
    let chain_count = chains.len();
    let draws = chains[0].len();
    let total = (chain_count * draws) as f64;
    let means: Vec<f64> = chains.iter().map(|chain| arithmetic_mean(chain)).collect();
    let within = chains
        .iter()
        .map(|chain| sample_variance(chain))
        .sum::<f64>()
        / chain_count as f64;
    let between = draws as f64 * sample_variance(&means);
    let variance = ((draws - 1) as f64 / draws as f64) * within + between / draws as f64;

    if variance <= f64::EPSILON {
        return total;
    }
    if within <= f64::EPSILON {
        return 1.0;
    }

    let autocovariances: Vec<Vec<f64>> = chains.iter().map(|chain| autocovariance(chain)).collect();
    let mut rhos = vec![1.0];
    for lag in 1..draws {
        let mean_autocov = autocovariances
            .iter()
            .map(|values| values[lag])
            .sum::<f64>()
            / chain_count as f64;
        rhos.push(1.0 - (within - mean_autocov) / variance);
    }

    let mut positive_sum = 0.0;
    let mut lag = 1;
    let mut previous_pair = f64::INFINITY;
    while lag < draws {
        let pair = if lag + 1 < draws {
            rhos[lag] + rhos[lag + 1]
        } else {
            rhos[lag]
        };
        if pair < 0.0 {
            break;
        }
        let monotone_pair = pair.min(previous_pair);
        positive_sum += monotone_pair;
        previous_pair = monotone_pair;
        lag += 2;
    }

    let tau = (-1.0 + 2.0 * (1.0 + positive_sum)).max(1.0 / total);
    (total / tau).clamp(1.0, total)
}

fn autocovariance(values: &[f64]) -> Vec<f64> {
    let mean = arithmetic_mean(values);
    (0..values.len())
        .map(|lag| {
            values[..values.len() - lag]
                .iter()
                .zip(&values[lag..])
                .map(|(left, right)| (left - mean) * (right - mean))
                .sum::<f64>()
                / values.len() as f64
        })
        .collect()
}

fn arithmetic_mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn sample_variance(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = arithmetic_mean(values);
    values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (values.len() - 1) as f64
}

fn quantile(values: &[f64], probability: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let position = probability * (sorted.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    if lower == upper {
        sorted[lower]
    } else {
        let fraction = position - lower as f64;
        sorted[lower] * (1.0 - fraction) + sorted[upper] * fraction
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn analyze(input: &str, thresholds: Thresholds) -> Result<DiagnosticsReport, Error> {
        analyze_reader(Cursor::new(input), thresholds)
    }

    #[test]
    fn identical_constant_chains_are_well_defined() {
        let input = (0..8)
            .flat_map(|draw| {
                ["a", "b"].map(move |chain| {
                    format!(r#"{{"chain":"{chain}","draw":{draw},"values":{{"x":2.0}}}}"#)
                })
            })
            .collect::<Vec<_>>()
            .join("\n");
        let report = analyze(
            &input,
            Thresholds {
                max_rhat: 1.01,
                min_ess: 1.0,
            },
        )
        .expect("constant chains should analyze");
        assert_eq!(report.variables["x"].rhat, 1.0);
        assert_eq!(report.variables["x"].bulk_ess, 16.0);
        assert!(report.converged);
    }

    #[test]
    fn separated_chains_fail_rhat() {
        let mut lines = Vec::new();
        for draw in 0..20 {
            lines.push(format!(
                r#"{{"chain":"a","draw":{draw},"values":{{"x":{}}}}}"#,
                draw as f64 / 100.0
            ));
            lines.push(format!(
                r#"{{"chain":"b","draw":{draw},"values":{{"x":{}}}}}"#,
                10.0 + draw as f64 / 100.0
            ));
        }
        let report = analyze(
            &lines.join("\n"),
            Thresholds {
                max_rhat: 1.01,
                min_ess: 1.0,
            },
        )
        .expect("valid data");
        assert!(!report.converged);
        assert!(report.variables["x"].rhat > 1.01);
        assert!(report.variables["x"].issues[0].starts_with("rhat"));
    }

    #[test]
    fn odd_draw_count_discards_one_middle_draw_per_chain() {
        let input = (0..5)
            .flat_map(|draw| {
                ["a", "b"].map(move |chain| {
                    format!(r#"{{"chain":"{chain}","draw":{draw},"values":{{"x":{draw}}}}}"#)
                })
            })
            .collect::<Vec<_>>()
            .join("\n");
        let report = analyze(
            &input,
            Thresholds {
                max_rhat: 10.0,
                min_ess: 1.0,
            },
        )
        .expect("valid data");
        assert_eq!(report.input.draws_per_chain, 5);
        assert_eq!(report.input.draws_used_per_chain, 4);
    }

    #[test]
    fn duplicate_draws_are_rejected() {
        let line = r#"{"chain":"a","draw":0,"values":{"x":1.0}}"#;
        let error = analyze(&format!("{line}\n{line}"), Thresholds::default())
            .expect_err("duplicate must fail");
        assert!(matches!(error, Error::DuplicateDraw { .. }));
    }

    #[test]
    fn variable_mismatch_is_rejected() {
        let input = r#"
{"chain":"a","draw":0,"values":{"x":1.0}}
{"chain":"a","draw":1,"values":{"x":1.0}}
{"chain":"a","draw":2,"values":{"x":1.0}}
{"chain":"a","draw":3,"values":{"y":1.0}}
{"chain":"b","draw":0,"values":{"x":1.0}}
{"chain":"b","draw":1,"values":{"x":1.0}}
{"chain":"b","draw":2,"values":{"x":1.0}}
{"chain":"b","draw":3,"values":{"x":1.0}}
"#;
        assert!(matches!(
            analyze(input, Thresholds::default()),
            Err(Error::VariableMismatch { .. })
        ));
    }
}
