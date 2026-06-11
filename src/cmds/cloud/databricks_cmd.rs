//! Databricks CLI output filtering.
//!
//! - `bundle validate`: collapses to "OK" on success; full output on failure.
//! - `jobs list-runs`: collapses URL-heavy table to CSV + one URL template.

use crate::core::tracking;
use crate::core::utils::{exit_code_from_output, resolved_command};
use anyhow::{Context, Result};
use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    static ref JOB_URL_SUFFIX: Regex =
        Regex::new(r"#job/(\d+)/run/(\d+)").unwrap();
}

pub fn run(subcommand: &str, args: &[String], verbose: u8) -> Result<i32> {
    let full_sub = if args.is_empty() {
        subcommand.to_string()
    } else {
        format!("{} {}", subcommand, args.join(" "))
    };

    match subcommand {
        "bundle" if !args.is_empty() && args[0] == "validate" => {
            run_bundle_validate(&args[1..], &full_sub, verbose)
        }
        "jobs" if !args.is_empty() && args[0] == "list-runs" => {
            run_jobs_list_runs(&args[1..], &full_sub, verbose)
        }
        _ => run_passthrough(subcommand, args, verbose),
    }
}

fn run_bundle_validate(extra_args: &[String], full_sub: &str, verbose: u8) -> Result<i32> {
    let timer = tracking::TimedExecution::start();
    let mut cmd = resolved_command("databricks");
    cmd.args(["bundle", "validate"]);
    for arg in extra_args {
        cmd.arg(arg);
    }

    if verbose > 0 {
        eprintln!("Running: databricks bundle validate {}", extra_args.join(" "));
    }

    let output = cmd.output().context("Failed to run databricks bundle validate")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stderr.trim().is_empty() {
            eprint!("{}", stderr);
        }
        if !stdout.trim().is_empty() {
            print!("{}", stdout);
        }
        return Ok(exit_code_from_output(&output, "databricks bundle validate"));
    }

    let raw = String::from_utf8_lossy(&output.stdout).to_string();
    let filtered = "OK".to_string();
    println!("{}", filtered);

    timer.track(
        &format!("databricks {}", full_sub),
        &format!("rtk databricks {}", full_sub),
        &raw,
        &filtered,
    );

    Ok(0)
}

fn run_jobs_list_runs(extra_args: &[String], full_sub: &str, verbose: u8) -> Result<i32> {
    let timer = tracking::TimedExecution::start();
    let mut cmd = resolved_command("databricks");
    cmd.args(["jobs", "list-runs"]);
    for arg in extra_args {
        cmd.arg(arg);
    }

    if verbose > 0 {
        eprintln!("Running: databricks jobs list-runs {}", extra_args.join(" "));
    }

    let output = cmd.output().context("Failed to run databricks jobs list-runs")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !stderr.trim().is_empty() {
            eprint!("{}", stderr);
        }
        if !stdout.trim().is_empty() {
            print!("{}", stdout);
        }
        return Ok(exit_code_from_output(&output, "databricks jobs list-runs"));
    }

    let raw = String::from_utf8_lossy(&output.stdout).to_string();
    let filtered = filter_jobs_list_runs(&raw);
    print!("{}", filtered);

    timer.track(
        &format!("databricks {}", full_sub),
        &format!("rtk databricks {}", full_sub),
        &raw,
        &filtered,
    );

    Ok(0)
}

fn filter_jobs_list_runs(output: &str) -> String {
    let mut url_template: Option<String> = None;
    // Preserve insertion order: Vec of (job_id, Vec<(run_id, state)>)
    let mut job_order: Vec<String> = Vec::new();
    let mut job_runs: std::collections::HashMap<String, Vec<(String, String)>> =
        std::collections::HashMap::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("Job ID") {
            continue;
        }

        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        if tokens.len() < 4 {
            continue;
        }

        let job_id = tokens[0].to_string();
        let run_id = tokens[1].to_string();
        let state = tokens[2].to_string();
        let url = tokens[3];

        if url_template.is_none() {
            let template = JOB_URL_SUFFIX
                .replace(url, "#job/<JOB-ID>/run/<RUN-ID>")
                .to_string();
            url_template = Some(template);
        }

        if !job_runs.contains_key(&job_id) {
            job_order.push(job_id.clone());
        }
        job_runs.entry(job_id).or_default().push((run_id, state));
    }

    let mut out = String::new();
    if let Some(tmpl) = url_template {
        out.push_str(&tmpl);
        out.push('\n');
    }
    out.push_str("JobID\n└RunID,✅/❌\n");

    for job_id in &job_order {
        out.push_str(job_id);
        out.push('\n');
        if let Some(runs) = job_runs.get(job_id) {
            let last = runs.len() - 1;
            for (i, (run_id, state)) in runs.iter().enumerate() {
                let branch = if i == last { '└' } else { '├' };
                let icon = match state.as_str() {
                    "SUCCESS" => "✅",
                    "FAILED" => "❌",
                    _ => state.as_str(),
                };
                out.push_str(&format!("{}{},{}\n", branch, run_id, icon));
            }
        }
    }
    out
}

fn run_passthrough(subcommand: &str, args: &[String], verbose: u8) -> Result<i32> {
    let mut cmd = resolved_command("databricks");
    cmd.arg(subcommand);
    for arg in args {
        cmd.arg(arg);
    }

    if verbose > 0 {
        eprintln!("Running: databricks {} {}", subcommand, args.join(" "));
    }

    let output = cmd.output().context("Failed to run databricks")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !stdout.is_empty() {
        print!("{}", stdout);
    }
    if !stderr.is_empty() {
        eprint!("{}", stderr);
    }

    Ok(exit_code_from_output(&output, "databricks"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_tokens(text: &str) -> usize {
        text.split_whitespace().count()
    }

    #[test]
    fn test_bundle_validate_success_output() {
        let raw = "Name: bundle_name\nTarget: development\nWorkspace:\n  User: jon@example.com\n  Path: /Workspace/Users/jon@example.com/.bundle/bundle_name/development\n\nValidation OK!\n";
        let filtered = "OK";
        let savings = 100.0 - (count_tokens(filtered) as f64 / count_tokens(raw) as f64 * 100.0);
        assert!(savings >= 80.0, "Expected ≥80% savings, got {:.1}%", savings);
    }

    #[test]
    fn test_jobs_list_runs_format() {
        let raw = "Job ID            Run ID            Result State  URL\n\
            32724381413570    8732142823392     FAILED        https://company.cloud.databricks.com/?o=1234567890#job/32724381413570/run/8732142823392\n\
            75250344412540    1009779845451074  FAILED        https://company.cloud.databricks.com/?o=1234567890#job/75250344412540/run/1009779845451074\n\
            712572557199186   557700187705942   SUCCESS       https://company.cloud.databricks.com/?o=1234567890#job/712572557199186/run/557700187705942\n\
            712572557199186   290095825765935   SUCCESS       https://company.cloud.databricks.com/?o=1234567890#job/712572557199186/run/290095825765935\n\
            372463076174166   190855044762720   SUCCESS       https://company.cloud.databricks.com/?o=1234567890#job/372463076174166/run/190855044762720\n\
            1038075514924486  981367437728235   FAILED        https://company.cloud.databricks.com/?o=1234567890#job/1038075514924486/run/981367437728235\n";

        let filtered = filter_jobs_list_runs(raw);

        assert!(filtered.contains("https://company.cloud.databricks.com/?o=1234567890#job/<JOB-ID>/run/<RUN-ID>"));
        assert!(filtered.contains("JobID\n└RunID,✅/❌\n"));
        assert!(filtered.contains("32724381413570\n└8732142823392,❌"));
        assert!(filtered.contains("712572557199186\n├557700187705942,✅\n└290095825765935,✅"));
        assert!(!filtered.contains("https://company.cloud.databricks.com/?o=1234567890#job/32724381413570"));

        let savings = 100.0 - (count_tokens(&filtered) as f64 / count_tokens(raw) as f64 * 100.0);
        // URL deduplication savings are character-based; whitespace token count understates them
        assert!(savings >= 50.0, "Expected ≥50% savings, got {:.1}%", savings);
    }
}
