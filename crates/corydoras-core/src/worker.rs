//! The background worker: one step takes the next due job and runs it.
//! Each phase goes to the Activity, so the app can show what happens now.

use crate::activity::Activity;
use crate::analysis;
use crate::library::{Library, JOB_ANALYZE, JOB_SOURCE, JOB_SYNTH};
use crate::models::Job;
use crate::provider::{self, CallError, Part, Provider, Thinking};
use crate::ai::{AiConfig, Role};
use crate::spec;
use crate::util::{now_ms, truncate};
use anyhow::{anyhow, Result};
use serde_json::Value;

/// Queue attempts for a job that fails with a transient error.
const MAX_ATTEMPTS: i64 = 3;
/// Answer budget of one image analysis (thinking is off).
const ITEM_TOKENS: u32 = 4_000;
/// Answer budget of one synthesis call, before the reasoning allowance.
const SYNTH_TOKENS: u32 = 24_000;
const SYNTH_TOKENS_MAX: u32 = 64_000;
/// Model calls in one synthesis job.
const SYNTH_CALLS: u32 = 3;

#[derive(Debug, Clone)]
pub enum Outcome {
    /// No job was due.
    Idle,
    /// A job ran (with success, failure or cancel). The UI must reload this language.
    Did { language_id: String, message: String },
    /// The worker cannot run jobs now (paused, no key, budget, database error).
    Paused(String),
}

fn secs_since(start_ms: i64) -> i64 {
    ((now_ms() - start_ms) as f64 / 1000.0).round() as i64
}

fn k(n: i64) -> String {
    if n >= 1000 {
        format!("{:.1} k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

pub async fn step(lib: &Library, act: &Activity) -> Outcome {
    match lib.queue_paused() {
        Ok(true) => return Outcome::Paused("The queue is paused.".into()),
        Err(e) => return Outcome::Paused(format!("Job queue error: {e}")),
        _ => {}
    }
    let manual = match lib.queue_manual() {
        Ok(m) => m,
        Err(e) => return Outcome::Paused(format!("Job queue error: {e}")),
    };
    let running_by_hand = manual && act.run_requested();
    if manual && !running_by_hand {
        return Outcome::Paused("Manual mode: jobs wait until you click Run queue.".into());
    }
    let job = match lib.next_job(running_by_hand) {
        Ok(Some(j)) => j,
        Ok(None) => {
            if running_by_hand && lib.job_count().unwrap_or(0) == 0 {
                act.set_run_requested(false);
                act.log("ok", None, "The queue is empty. Manual mode waits for your next Run queue.");
            }
            return Outcome::Idle;
        }
        Err(e) => return Outcome::Paused(format!("Job queue error: {e}")),
    };
    let settings = match lib.settings() {
        Ok(s) => s,
        Err(e) => return Outcome::Paused(format!("Settings error: {e}")),
    };
    if settings.monthly_token_budget > 0 {
        if let Ok(u) = lib.usage() {
            if u.input_tokens + u.output_tokens >= settings.monthly_token_budget {
                let _ = lib.defer_job(&job, 300);
                return Outcome::Paused(
                    "The monthly token budget is used up. Raise it in Settings to continue.".into(),
                );
            }
        }
    }
    let ai = AiConfig::effective(crate::config::AppConfig::load().ai.as_ref(), &settings);
    let role = if job.kind == JOB_SYNTH { Role::Synthesis } else { Role::Analysis };
    let Some(profile) = ai.for_role(role) else {
        let _ = lib.defer_job(&job, 20);
        return Outcome::Paused("Add a model provider in Settings to start the analysis.".into());
    };
    let key = match profile.key() {
        Ok(k) => k,
        Err(e) => {
            let _ = lib.defer_job(&job, 20);
            return Outcome::Paused(e.to_string());
        }
    };
    let provider = match Provider::from_profile(profile, &key) {
        Ok(p) => p,
        Err(e) => {
            let _ = lib.defer_job(&job, 20);
            return Outcome::Paused(e.to_string());
        }
    };
    let thinking = Thinking::parse(&profile.thinking);
    match job.kind.as_str() {
        JOB_ANALYZE => run_analyze(lib, act, &provider, &job).await,
        JOB_SOURCE => run_source(lib, act, &provider, &job).await,
        JOB_SYNTH => run_synth(lib, act, &provider, &job, thinking).await,
        _ => {
            let _ = lib.drop_job(&job);
            Outcome::Idle
        }
    }
}

/// Records a failure. Transient errors go back in the queue with a delay;
/// other errors stop the job, because the same call would fail the same way.
/// Returns a sentence about what happens next.
fn fail(lib: &Library, job: &Job, e: &anyhow::Error) -> (bool, String) {
    let msg = e.to_string();
    if provider::is_transient(e) && job.attempts + 1 < MAX_ATTEMPTS {
        let delay = 30 * (1 << job.attempts.clamp(0, 4));
        if lib.retry_job(job, delay, &msg).is_ok() {
            return (
                false,
                format!("The queue tries again in {delay} s (try {} of {MAX_ATTEMPTS}).", job.attempts + 2),
            );
        }
    }
    let _ = lib.drop_job(job);
    (true, "The queue stopped this job.".into())
}

async fn run_analyze(lib: &Library, act: &Activity, provider: &Provider, job: &Job) -> Outcome {
    let item = match lib.get_item(&job.target) {
        Ok(i) => i,
        Err(_) => {
            let _ = lib.drop_job(job);
            return Outcome::Idle;
        }
    };
    let lang_id = item.language_id.clone();
    let lang_name = lib.get_language(&lang_id).map(|l| l.name).unwrap_or_default();
    act.begin(
        job.id,
        JOB_ANALYZE,
        &lang_id,
        &lang_name,
        Some(&item.id),
        Some(&item.original_name),
        &format!("Analyzing \"{}\"", item.original_name),
    );
    let _ = lib.set_item_status(&item.id, "analyzing", None);
    let start = now_ms();
    let outcome = match analyze_item(lib, provider, &item.id, act).await {
        Ok((i, o)) => {
            let _ = lib.complete_job(job);
            let _ = lib.enqueue_synth(&lang_id);
            act.log(
                "ok",
                Some(&lang_id),
                format!("Analyzed \"{}\" in {} s ({} in, {} out tokens).", item.original_name, secs_since(start), k(i), k(o)),
            );
            Outcome::Did { language_id: lang_id, message: "Item analyzed".into() }
        }
        Err(e) if provider::is_cancelled(&e) => {
            let _ = lib.drop_job(job);
            let _ = lib.mark_item_cancelled(&item.id);
            act.log("warn", Some(&lang_id), format!("Cancelled the analysis of \"{}\".", item.original_name));
            Outcome::Did { language_id: lang_id, message: "Cancelled".into() }
        }
        Err(e) => {
            let (gave_up, next) = fail(lib, job, &e);
            let msg = e.to_string();
            let _ = lib.set_item_status(&item.id, if gave_up { "error" } else { "pending" }, Some(&msg));
            act.log("error", Some(&lang_id), format!("Analysis of \"{}\" failed: {msg} {next}", item.original_name));
            Outcome::Did { language_id: lang_id, message: format!("Analysis failed: {msg}") }
        }
    };
    act.end();
    outcome
}

async fn run_synth(lib: &Library, act: &Activity, provider: &Provider, job: &Job, thinking: Thinking) -> Outcome {
    let lang_id = job.target.clone();
    let Ok(language) = lib.get_language(&lang_id) else {
        let _ = lib.drop_job(job);
        return Outcome::Idle;
    };
    match lib.pending_analyze_count(&lang_id) {
        Ok(n) if n > 0 => {
            // Wait for pass 1 to finish.
            let _ = lib.defer_job(job, 10);
            return Outcome::Idle;
        }
        Err(e) => return Outcome::Paused(format!("Job queue error: {e}")),
        _ => {}
    }
    act.begin(
        job.id,
        JOB_SYNTH,
        &lang_id,
        &language.name,
        None,
        None,
        &format!("Writing the language document for {}", language.name),
    );
    let start = now_ms();
    let outcome = match synthesize(lib, provider, &lang_id, thinking, act).await {
        Ok(Some(v)) => {
            let _ = lib.complete_job(job);
            Outcome::Did { language_id: lang_id, message: format!("Version {v} written") }
        }
        Ok(None) => {
            let _ = lib.complete_job(job);
            Outcome::Did { language_id: lang_id, message: "No change since the last version".into() }
        }
        Err(e) if provider::is_cancelled(&e) => {
            let _ = lib.drop_job(job);
            act.log(
                "warn",
                Some(&lang_id),
                format!("Cancelled writing the document for {} after {} s. No version was saved.", language.name, secs_since(start)),
            );
            Outcome::Did { language_id: lang_id, message: "Cancelled".into() }
        }
        Err(e) => {
            let (_, next) = fail(lib, job, &e);
            let msg = e.to_string();
            act.log("error", Some(&lang_id), format!("Writing the document for {} failed: {msg} {next}", language.name));
            Outcome::Did { language_id: lang_id, message: format!("Synthesis failed: {msg}") }
        }
    };
    act.end();
    outcome
}

async fn run_source(lib: &Library, act: &Activity, provider: &Provider, job: &Job) -> Outcome {
    let src = match lib.get_source(&job.target) {
        Ok(s) => s,
        Err(_) => {
            let _ = lib.drop_job(job);
            return Outcome::Idle;
        }
    };
    let lang_id = src.language_id.clone();
    let lang_name = lib.get_language(&lang_id).map(|l| l.name).unwrap_or_default();
    act.begin(job.id, JOB_SOURCE, &lang_id, &lang_name, Some(&src.id), Some(&src.title), &format!("Reading \"{}\"", src.title));
    let _ = lib.set_source_status(&src.id, "analyzing", None);
    let start = now_ms();
    let outcome = match analyze_source(lib, provider, &src.id, act).await {
        Ok((i, o)) => {
            let _ = lib.complete_job(job);
            let _ = lib.enqueue_synth(&lang_id);
            act.log("ok", Some(&lang_id), format!("Read \"{}\" in {} s ({} in, {} out tokens).", src.title, secs_since(start), k(i), k(o)));
            Outcome::Did { language_id: lang_id, message: "Source analyzed".into() }
        }
        Err(e) if provider::is_cancelled(&e) => {
            let _ = lib.drop_job(job);
            let _ = lib.mark_source_cancelled(&src.id);
            act.log("warn", Some(&lang_id), format!("Cancelled reading \"{}\".", src.title));
            Outcome::Did { language_id: lang_id, message: "Cancelled".into() }
        }
        Err(e) => {
            let (gave_up, next) = fail(lib, job, &e);
            let msg = e.to_string();
            let _ = lib.set_source_status(&src.id, if gave_up { "error" } else { "pending" }, Some(&msg));
            act.log("error", Some(&lang_id), format!("Reading \"{}\" failed: {msg} {next}", src.title));
            Outcome::Did { language_id: lang_id, message: format!("Source analysis failed: {msg}") }
        }
    };
    act.end();
    outcome
}

/// Pass 1 for one source (text only). Returns the (input, output) tokens.
pub async fn analyze_source(lib: &Library, provider: &Provider, source_id: &str, act: &Activity) -> Result<(i64, i64)> {
    let src = lib.get_source(source_id)?;
    let language = lib.get_language(&src.language_id)?;
    act.phase("Preparing the text");
    let text = lib.source_text(source_id)?;
    // A PDF with little or no text goes in as page images.
    let mut images: Vec<String> = Vec::new();
    let mut pages = None;
    if src.media == "pdf" {
        let file = src.file_rel.as_deref().and_then(|r| lib.abs(r).ok()).filter(|p| p.exists());
        if let Some(path) = file {
            let bytes = std::fs::read(&path)?;
            let chars = text.chars().count() as i64;
            let rendered = tokio::task::spawn_blocking(move || {
                crate::sources::pdf_pages(&bytes, crate::sources::PDF_IMAGE_PAGES, crate::sources::PDF_PAGE_SIDE)
            })
            .await;
            match rendered {
                Ok(Ok((jpegs, total))) if !jpegs.is_empty() => {
                    if chars < crate::sources::PDF_SPARSE_CHARS_PER_PAGE * total as i64 {
                        act.phase(&format!("Drawing {} PDF page{}", jpegs.len(), if jpegs.len() == 1 { "" } else { "s" }));
                        pages = Some((jpegs.len(), total));
                        images = jpegs.iter().map(|j| analysis::data_url_jpeg(j)).collect();
                    }
                }
                Ok(Err(e)) if text.trim().is_empty() => return Err(e),
                Ok(Err(e)) => act.log("warn", Some(&src.language_id), format!("Could not draw the pages of \"{}\": {e} Using the text only.", src.title)),
                _ => {}
            }
        }
    }
    let mut parts = vec![Part::Text(analysis::source_user_text(&language, &src, &text, pages))];
    parts.extend(images.into_iter().map(Part::Image));
    let (mut tin, mut tout) = (0, 0);
    let mut budget = ITEM_TOKENS;
    for attempt in 1..=2u32 {
        act.call_begin(attempt, 2, "Waiting for the model", budget as u64);
        let r = provider
            .chat(analysis::SOURCE_SYSTEM, &parts, true, budget, Thinking::Off, Some(act))
            .await?;
        act.call_end();
        lib.add_usage(r.input_tokens, r.output_tokens)?;
        tin += r.input_tokens;
        tout += r.output_tokens;
        if r.cut_off() && attempt == 1 {
            act.log("warn", Some(&src.language_id), format!("The notes on \"{}\" were cut off. Trying again with a larger limit.", src.title));
            budget *= 2;
            continue;
        }
        if act.token().is_cancelled() {
            return Err(CallError::Cancelled.into());
        }
        act.phase("Saving the notes");
        let value = analysis::parse_json_loose(&r.text)?;
        lib.save_source_analysis(source_id, &value, &provider.model)?;
        return Ok((tin, tout));
    }
    Err(anyhow!("The source notes were cut off at the token limit two times."))
}

/// Pass 1 for one item. Returns the (input, output) tokens.
pub async fn analyze_item(lib: &Library, provider: &Provider, item_id: &str, act: &Activity) -> Result<(i64, i64)> {
    let item = lib.get_item(item_id)?;
    let regions = lib.get_regions(item_id)?;
    let language = lib.get_language(&item.language_id)?;
    let original = lib.abs(&item.file_rel)?;
    let crops_dir = lib.lang_dir(&item.language_id).join("crops");
    act.phase(if regions.is_empty() {
        "Preparing the image".to_string()
    } else {
        format!("Preparing the image and {} marked detail{}", regions.len(), if regions.len() == 1 { "" } else { "s" })
    }.as_str());
    let images = analysis::prepare_item_images(&original, &crops_dir, &item.id, &regions)?;

    let mut parts = vec![Part::Text(analysis::item_user_text(&language, &item, &regions))];
    parts.extend(images.into_iter().map(Part::Image));

    let (mut tin, mut tout) = (0, 0);
    let mut budget = ITEM_TOKENS;
    for attempt in 1..=2u32 {
        act.call_begin(attempt, 2, "Waiting for the model", budget as u64);
        // Thinking is off: the item pass describes what it sees, and reasoning tokens would multiply the cost per image.
        let r = provider
            .chat(analysis::ITEM_SYSTEM, &parts, true, budget, Thinking::Off, Some(act))
            .await?;
        act.call_end();
        lib.add_usage(r.input_tokens, r.output_tokens)?;
        tin += r.input_tokens;
        tout += r.output_tokens;
        if r.cut_off() && attempt == 1 {
            act.log(
                "warn",
                Some(&item.language_id),
                format!("The analysis of \"{}\" was cut off at {} tokens. Trying again with a larger limit.", item.original_name, budget),
            );
            budget *= 2;
            continue;
        }
        if act.token().is_cancelled() {
            return Err(CallError::Cancelled.into());
        }
        act.phase("Saving the analysis");
        let value = analysis::parse_json_loose(&r.text)?;
        lib.save_analysis(item_id, &value, &provider.model)?;
        return Ok((tin, tout));
    }
    Err(anyhow!("The analysis was cut off at the token limit two times."))
}

/// Pass 2 for one language. Returns the new version number, or None when the
/// inputs did not change since the last version.
pub async fn synthesize(
    lib: &Library,
    provider: &Provider,
    language_id: &str,
    thinking: Thinking,
    act: &Activity,
) -> Result<Option<i64>> {
    let language = lib.get_language(language_id)?;
    act.phase("Collecting the analyses");
    let inputs = lib.synthesis_inputs(language_id, analysis::SYNTH_ITEM_LIMIT)?;
    let srcs = lib.source_inputs(language_id, analysis::SYNTH_SOURCE_LIMIT)?;
    let lang = Some(language_id);
    if inputs.is_empty() {
        act.log("info", lang, format!("{}: nothing to write yet, because no image has an analysis. Sources need at least one image.", language.name));
        return Ok(None);
    }
    let hash = analysis::input_hash(&language, &inputs, &srcs);
    if lib.latest_input_hash(language_id)?.as_deref() == Some(hash.as_str()) {
        act.log("info", lang, format!("{}: no change since the last version, so no new version.", language.name));
        return Ok(None);
    }
    let previous = lib.read_latest(language_id)?;
    let base_text = analysis::synth_user_text(&language, &inputs, &srcs, previous.as_ref());
    let mut known: Vec<String> = inputs.iter().map(|i| i.item.id.chars().take(8).collect()).collect();
    known.extend(srcs.iter().map(|x| x.source.id.chars().take(8).collect::<String>()));
    let ids: Vec<String> = inputs.iter().map(|i| i.item.id.clone()).collect();

    let mut think = thinking;
    let mut budget = SYNTH_TOKENS;
    let mut text = base_text.clone();
    let mut label = "Waiting for the model".to_string();
    let mut best: Option<(Value, Vec<String>)> = None;
    let mut review_used = false;
    let mut last_err = anyhow!("The synthesis did not run.");

    for call in 1..=SYNTH_CALLS {
        act.call_begin(call, SYNTH_CALLS, &label, (budget + think.token_allowance()) as u64);
        act.log(
            "info",
            lang,
            format!(
                "{}: attempt {call} of {SYNTH_CALLS}, {} image and {} source analyses (about {} input tokens), thinking {}.",
                language.name,
                inputs.len(),
                srcs.len(),
                k((text.len() / 4) as i64),
                think.label()
            ),
        );
        let r = provider
            .chat(analysis::SYNTH_SYSTEM, &[Part::Text(text.clone())], true, budget, think, Some(act))
            .await?;
        act.call_end();
        lib.add_usage(r.input_tokens, r.output_tokens)?;

        if r.cut_off() {
            let limit = (budget + think.token_allowance()) / 1000;
            last_err = anyhow!("The answer was cut off at the token limit ({limit} k tokens).");
            text = format!(
                "{base_text}\n\nYOUR PREVIOUS ANSWER WAS CUT OFF AT THE TOKEN LIMIT. Keep the spec compact: fewer families, measures and rules, and short texts. Return the complete json object."
            );
            if think != Thinking::Off {
                act.log("warn", lang, format!("Attempt {call}: the answer was cut off at {limit} k tokens. The next attempt turns thinking off, so every token goes to the answer."));
                think = Thinking::Off;
                label = "Retrying without thinking".into();
            } else if budget < SYNTH_TOKENS_MAX {
                budget = (budget * 2).min(SYNTH_TOKENS_MAX);
                act.log("warn", lang, format!("Attempt {call}: the answer was cut off at {limit} k tokens. The next attempt allows {} k.", budget / 1000));
                label = "Retrying with a larger token limit".into();
            } else {
                act.log("error", lang, format!("Attempt {call}: the answer is too long even at {limit} k tokens."));
                last_err = anyhow!("The answer is too long even at {limit} k tokens. Remove some items or split the language.");
                break;
            }
            continue;
        }

        act.phase("Checking the answer");
        let mut value = match analysis::parse_json_loose(&r.text) {
            Ok(v) => v,
            Err(e) => {
                act.log("warn", lang, format!("Attempt {call}: the answer is not valid JSON ({e}). Asking again."));
                text = format!("{base_text}\n\nYOUR PREVIOUS ANSWER WAS NOT VALID JSON: {e}\nReturn the complete json object.");
                label = "Asking for valid JSON".into();
                last_err = e;
                continue;
            }
        };
        let rep = spec::repair(&mut value);
        if !rep.fixed.is_empty() {
            act.log(
                "info",
                lang,
                format!("Attempt {call}: fixed {} small format problem{} automatically.", rep.fixed.len(), if rep.fixed.len() == 1 { "" } else { "s" }),
            );
        }
        let check = spec::validate(&value, &known);
        if !check.errors.is_empty() {
            let n = check.errors.len();
            act.log(
                "warn",
                lang,
                format!("Attempt {call}: {n} validation error{}, for example: {} Asking the model to fix them.", if n == 1 { "" } else { "s" }, truncate(&check.errors[0], 160)),
            );
            let list = check.errors.iter().map(|e| format!("- {e}")).collect::<Vec<_>>().join("\n");
            text = format!("{base_text}\n\nYOUR PREVIOUS ANSWER FAILED VALIDATION:\n{list}\nReturn the complete, corrected json object.");
            label = format!("Fixing {n} validation error{}", if n == 1 { "" } else { "s" });
            last_err = anyhow!(
                "The answer failed validation {} time{}: {}",
                call,
                if call == 1 { "" } else { "s" },
                check.errors.iter().take(3).cloned().collect::<Vec<_>>().join(" ")
            );
            continue;
        }
        let mut warnings = rep.flagged.clone();
        warnings.extend(check.warnings.into_iter());
        let better = best.as_ref().map(|(_, w)| warnings.len() < w.len()).unwrap_or(true);
        if better {
            best = Some((value, warnings.clone()));
        }
        if warnings.is_empty() || review_used || call == SYNTH_CALLS {
            break;
        }
        review_used = true;
        let n = warnings.len();
        act.log("info", lang, format!("Attempt {call}: the review found {n} note{}. Asking the model to fix them.", if n == 1 { "" } else { "s" }));
        let list = warnings.iter().map(|w| format!("- {w}")).collect::<Vec<_>>().join("\n");
        text = format!(
            "{base_text}\n\nYOUR PREVIOUS ANSWER:\n{}\n\nAN AUTOMATIC REVIEW FOUND THESE PROBLEMS:\n{list}\nFix them. Move numbers from text into measures, and cite only valid evidence IDs. Return the complete, corrected json object.",
            serde_json::to_string(&best.as_ref().map(|b| b.0.clone()).unwrap_or_default()).unwrap_or_default()
        );
        label = format!("Fixing {n} review note{}", if n == 1 { "" } else { "s" });
    }

    if act.token().is_cancelled() {
        return Err(CallError::Cancelled.into());
    }
    if let Some((value, warnings)) = best {
        act.phase("Saving the new version");
        let v = lib.save_spec_version(language_id, &value, &provider.model, &ids, false, &hash, &warnings)?;
        act.log(
            "ok",
            lang,
            if warnings.is_empty() {
                format!("{}: version {v} saved.", language.name)
            } else {
                format!("{}: version {v} saved with {} review note{}.", language.name, warnings.len(), if warnings.len() == 1 { "" } else { "s" })
            },
        );
        return Ok(Some(v));
    }
    Err(last_err)
}
