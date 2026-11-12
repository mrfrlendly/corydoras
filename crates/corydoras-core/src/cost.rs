//! Cost: an estimate of the tokens (and the money, when the provider has
//! prices) that a pass will use, before it runs. Also the check for analyses
//! that an earlier model wrote.
//!
//! The estimate uses the tokens of earlier calls of the same kind in this
//! library. Without earlier calls, it uses typical values.

use crate::ai::{AiConfig, Role};
use crate::analysis;
use crate::library::{Library, JOB_ANALYZE, JOB_SOURCE, JOB_SYNTH};
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

/// Typical tokens of one image analysis when the library has no history:
/// the instructions and the answer shape, plus one image of 2048 px.
const ITEM_INPUT: i64 = 2_800;
const ITEM_OUTPUT: i64 = 1_100;
/// Each marked region adds one crop.
const REGION_INPUT: i64 = 900;
const SOURCE_OUTPUT: i64 = 1_200;
const SYNTH_OUTPUT: i64 = 7_000;
/// Characters of English text for each token.
const CHARS_PER_TOKEN: usize = 4;

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Pass {
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Estimate {
    /// Image analyses.
    pub images: Pass,
    /// Source analyses.
    pub sources: Pass,
    /// The language document.
    pub document: Pass,
    pub total_tokens: i64,
    /// In US dollars. None when a provider that the passes use has no prices.
    pub cost_usd: Option<f64>,
    /// True when the numbers come from earlier calls in this library.
    pub from_history: bool,
    /// Tokens left in the monthly budget. None: no budget.
    pub budget_left: Option<i64>,
    /// True when a pass of this estimate uses the ChatGPT plan of the person.
    pub plan: bool,
}

/// What to estimate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The jobs that wait in the queue for this language.
    Queue,
    /// Every image and source again, then the document.
    All,
    /// Only the document.
    Document,
    /// The analyses that another model wrote, then the document.
    OtherModel,
}

impl Scope {
    pub fn parse(s: &str) -> Scope {
        match s {
            "all" => Scope::All,
            "document" => Scope::Document,
            "other-model" => Scope::OtherModel,
            _ => Scope::Queue,
        }
    }
}

/// Analyses of a language that a model other than `model` wrote.
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct OtherModel {
    pub images: i64,
    pub sources: i64,
    /// The models that wrote them.
    pub models: Vec<String>,
}

impl Library {
    /// Counts the tokens of one model call: in the monthly usage and in the
    /// averages of its kind.
    pub fn add_usage_kind(&self, kind: &str, input: i64, output: i64) -> Result<()> {
        self.add_usage(input, output)?;
        self.c().execute(
            "INSERT INTO job_stats (kind, calls, input_tokens, output_tokens) VALUES (?1, 1, ?2, ?3)
             ON CONFLICT(kind) DO UPDATE SET calls = calls + 1, input_tokens = input_tokens + ?2, output_tokens = output_tokens + ?3",
            params![kind, input, output],
        )?;
        Ok(())
    }

    /// (calls, average input, average output) of one kind of call.
    fn job_average(&self, kind: &str) -> Result<Option<(i64, i64, i64)>> {
        let row: Option<(i64, i64, i64)> = self
            .c()
            .query_row("SELECT calls, input_tokens, output_tokens FROM job_stats WHERE kind = ?1", [kind], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        Ok(row.filter(|r| r.0 >= 3).map(|(n, i, o)| (n, i / n, o / n)))
    }

    /// The analyses of a language that another model wrote.
    pub fn other_model_analyses(&self, language_id: &str, model: &str) -> Result<OtherModel> {
        let c = self.c();
        let images: i64 = c.query_row(
            "SELECT COUNT(*) FROM analyses a JOIN items i ON i.id = a.item_id WHERE i.language_id = ?1 AND a.model <> ?2",
            params![language_id, model],
            |r| r.get(0),
        )?;
        let sources: i64 = c.query_row(
            "SELECT COUNT(*) FROM sources WHERE language_id = ?1 AND analysis IS NOT NULL AND COALESCE(analysis_model, '') <> ?2",
            params![language_id, model],
            |r| r.get(0),
        )?;
        let mut st = c.prepare(
            "SELECT DISTINCT a.model FROM analyses a JOIN items i ON i.id = a.item_id WHERE i.language_id = ?1 AND a.model <> ?2
             UNION SELECT DISTINCT analysis_model FROM sources WHERE language_id = ?1 AND analysis IS NOT NULL AND COALESCE(analysis_model, '') <> ?2",
        )?;
        let mut models: Vec<String> = st.query_map(params![language_id, model], |r| r.get::<_, Option<String>>(0))?.flatten().flatten().filter(|m| !m.is_empty()).collect();
        models.sort();
        Ok(OtherModel { images, sources, models })
    }

    /// Queues the analyses that another model wrote. Returns the count.
    pub fn reanalyze_other_model(&self, language_id: &str, model: &str) -> Result<usize> {
        let (items, sources): (Vec<String>, Vec<String>) = {
            let c = self.c();
            let mut st = c.prepare("SELECT a.item_id FROM analyses a JOIN items i ON i.id = a.item_id WHERE i.language_id = ?1 AND a.model <> ?2")?;
            let items = st.query_map(params![language_id, model], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            let mut st = c.prepare("SELECT id FROM sources WHERE language_id = ?1 AND analysis IS NOT NULL AND COALESCE(analysis_model, '') <> ?2")?;
            let sources = st.query_map(params![language_id, model], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
            (items, sources)
        };
        for id in &items {
            self.set_item_status(id, "pending", None)?;
            self.enqueue(JOB_ANALYZE, id, 0)?;
        }
        for id in &sources {
            self.set_source_status(id, "pending", None)?;
            self.enqueue(JOB_SOURCE, id, 0)?;
        }
        if !items.is_empty() || !sources.is_empty() {
            self.enqueue_synth(language_id)?;
        }
        Ok(items.len() + sources.len())
    }
}

/// The estimate for one language.
pub fn estimate(lib: &Library, ai: &AiConfig, language_id: &str, scope: Scope) -> Result<Estimate> {
    let lang = lib.get_language(language_id)?;
    let items = lib.list_items(language_id)?;
    let sources = lib.list_sources(language_id)?;
    let analysis_model = ai.for_role(Role::Analysis).map(|p| p.model.clone()).unwrap_or_default();

    // Which items and sources get a call.
    let queued: Vec<(String, String)> = lib.list_jobs()?.into_iter().filter(|j| j.language_id == language_id).map(|j| (j.kind, j.target)).collect();
    let stale_items: Vec<String> = if scope == Scope::OtherModel {
        let c = lib.c();
        let mut st = c.prepare("SELECT a.item_id FROM analyses a JOIN items i ON i.id = a.item_id WHERE i.language_id = ?1 AND a.model <> ?2")?;
        let rows = st.query_map(params![language_id, analysis_model], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        rows
    } else {
        Vec::new()
    };
    let item_calls: Vec<&crate::models::Item> = items
        .iter()
        .filter(|i| match scope {
            Scope::All => true,
            Scope::Document => false,
            Scope::Queue => queued.iter().any(|(k, t)| k == JOB_ANALYZE && t == &i.id),
            Scope::OtherModel => stale_items.contains(&i.id),
        })
        .collect();
    let stale_sources = if scope == Scope::OtherModel { lib.other_model_analyses(language_id, &analysis_model)?.sources } else { 0 };
    let source_calls: Vec<&crate::models::Source> = sources
        .iter()
        .filter(|x| match scope {
            Scope::All => true,
            Scope::Document => false,
            Scope::Queue => queued.iter().any(|(k, t)| k == JOB_SOURCE && t == &x.id),
            Scope::OtherModel => stale_sources > 0 && x.analysis_status == "done",
        })
        .collect();
    let writes_document = lang.base_id.is_none()
        && !items.is_empty()
        && match scope {
            Scope::Queue => queued.iter().any(|(k, _)| k == JOB_SYNTH),
            Scope::OtherModel => !item_calls.is_empty() || !source_calls.is_empty(),
            _ => true,
        };

    let mut e = Estimate::default();
    let item_avg = lib.job_average(JOB_ANALYZE)?;
    let source_avg = lib.job_average(JOB_SOURCE)?;
    let synth_avg = lib.job_average(JOB_SYNTH)?;
    e.from_history = (item_calls.is_empty() || item_avg.is_some()) && (source_calls.is_empty() || source_avg.is_some()) && (!writes_document || synth_avg.is_some());

    for i in &item_calls {
        e.images.calls += 1;
        match item_avg {
            Some((_, input, output)) => {
                e.images.input_tokens += input;
                e.images.output_tokens += output;
            }
            None => {
                e.images.input_tokens += ITEM_INPUT + i.region_count * REGION_INPUT;
                e.images.output_tokens += ITEM_OUTPUT;
            }
        }
    }
    for x in &source_calls {
        e.sources.calls += 1;
        match source_avg {
            Some((_, input, output)) => {
                e.sources.input_tokens += input;
                e.sources.output_tokens += output;
            }
            None => {
                let text = (x.text_chars as usize).min(analysis::SOURCE_TEXT_LIMIT) / CHARS_PER_TOKEN;
                e.sources.input_tokens += 900 + text as i64;
                e.sources.output_tokens += SOURCE_OUTPUT;
            }
        }
    }
    if writes_document {
        // The real prompt of the document pass, built from the analyses that exist now.
        let inputs = lib.synthesis_inputs(language_id, analysis::SYNTH_ITEM_LIMIT)?;
        let srcs = lib.source_inputs(language_id, analysis::SYNTH_SOURCE_LIMIT)?;
        let previous = lib.read_latest(language_id)?;
        let text = analysis::synth_user_text(&lang, &inputs, &srcs, previous.as_ref());
        let mut input = ((text.len() + analysis::SYNTH_SYSTEM.len()) / CHARS_PER_TOKEN) as i64;
        // Items that have no analysis yet add their notes to the prompt later.
        let missing = item_calls.iter().filter(|i| !inputs.iter().any(|x| x.item.id == i.id)).count() as i64;
        input += missing * 700;
        e.document = Pass { calls: 1, input_tokens: input, output_tokens: synth_avg.map(|a| a.2).unwrap_or(SYNTH_OUTPUT) };
    }
    e.total_tokens = [&e.images, &e.sources, &e.document].iter().map(|p| p.input_tokens + p.output_tokens).sum();

    // Money, when the providers have prices.
    let price = |role: Role, passes: &[&Pass]| -> Option<f64> {
        let p = ai.for_role(role)?;
        let tokens_in: i64 = passes.iter().map(|x| x.input_tokens).sum();
        let tokens_out: i64 = passes.iter().map(|x| x.output_tokens).sum();
        if tokens_in + tokens_out == 0 {
            return Some(0.0);
        }
        if p.local() {
            return Some(0.0);
        }
        // The ChatGPT plan pays: tokens have no price here.
        if p.uses_account() {
            return None;
        }
        if p.price_in <= 0.0 && p.price_out <= 0.0 {
            return None;
        }
        Some(tokens_in as f64 / 1e6 * p.price_in + tokens_out as f64 / 1e6 * p.price_out)
    };
    e.cost_usd = price(Role::Analysis, &[&e.images, &e.sources]).zip(price(Role::Synthesis, &[&e.document])).map(|(a, b)| ((a + b) * 10_000.0).round() / 10_000.0);

    let on_plan = |role: Role, passes: &[&Pass]| ai.for_role(role).is_some_and(|p| p.uses_account()) && passes.iter().any(|x| x.calls > 0);
    e.plan = on_plan(Role::Analysis, &[&e.images, &e.sources]) || on_plan(Role::Synthesis, &[&e.document]);

    let settings = lib.settings()?;
    if settings.monthly_token_budget > 0 {
        let u = lib.usage()?;
        e.budget_left = Some((settings.monthly_token_budget - u.input_tokens - u.output_tokens).max(0));
    }
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::ProviderProfile;
    use serde_json::json;

    fn png(seed: u8) -> Vec<u8> {
        let img = image::RgbImage::from_fn(40, 30, |x, y| image::Rgb([(x as u8).wrapping_mul(seed), y as u8, seed]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img).write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    fn ai(price_in: f64, price_out: f64) -> AiConfig {
        let mut p = ProviderProfile::from_preset("openai");
        p.model = "model-b".into();
        p.price_in = price_in;
        p.price_out = price_out;
        AiConfig { analysis_profile: p.id.clone(), synthesis_profile: p.id.clone(), profiles: vec![p] }
    }

    #[test]
    fn estimates_and_other_models() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let lang = lib.create_language("Organic", "").unwrap();
        let a = lib.import_bytes(&lang.id, "a.png", &png(3), None).unwrap();
        let b = lib.import_bytes(&lang.id, "b.png", &png(5), None).unwrap();

        // The queue has two image analyses and the document.
        let e = estimate(&lib, &ai(0.0, 0.0), &lang.id, Scope::Queue).unwrap();
        assert_eq!(e.images, Pass { calls: 2, input_tokens: 2 * ITEM_INPUT, output_tokens: 2 * ITEM_OUTPUT });
        assert_eq!(e.document.calls, 1);
        assert!(e.document.input_tokens > 1_000, "the prompt of the document pass counts: {}", e.document.input_tokens);
        assert!(!e.from_history);
        assert_eq!(e.cost_usd, None, "no prices, no money");
        assert_eq!(e.total_tokens, e.images.input_tokens + e.images.output_tokens + e.document.input_tokens + e.document.output_tokens);
        assert!(e.budget_left.unwrap() > 0);

        // With prices: dollars per million tokens.
        let e = estimate(&lib, &ai(1.0, 4.0), &lang.id, Scope::Queue).unwrap();
        let want = (e.images.input_tokens + e.document.input_tokens) as f64 / 1e6 + (e.images.output_tokens + e.document.output_tokens) as f64 / 1e6 * 4.0;
        assert!((e.cost_usd.unwrap() - want).abs() < 1e-3, "{:?} {want}", e.cost_usd);

        // Earlier calls give the averages.
        for _ in 0..3 {
            lib.add_usage_kind(JOB_ANALYZE, 3000, 900).unwrap();
        }
        let e = estimate(&lib, &ai(0.0, 0.0), &lang.id, Scope::All).unwrap();
        assert_eq!((e.images.calls, e.images.input_tokens, e.images.output_tokens), (2, 6000, 1800));
        assert_eq!(lib.usage().unwrap().requests, 3);

        // Analyses of another model.
        lib.save_analysis(&a.id, &json!({"summary": "x"}), "model-a").unwrap();
        lib.save_analysis(&b.id, &json!({"summary": "y"}), "model-b").unwrap();
        let other = lib.other_model_analyses(&lang.id, "model-b").unwrap();
        assert_eq!(other, OtherModel { images: 1, sources: 0, models: vec!["model-a".into()] });
        lib.cancel_jobs(None).unwrap();
        let e = estimate(&lib, &ai(0.0, 0.0), &lang.id, Scope::OtherModel).unwrap();
        assert_eq!((e.images.calls, e.document.calls), (1, 1));
        assert_eq!(estimate(&lib, &ai(0.0, 0.0), &lang.id, Scope::Queue).unwrap().total_tokens, 0);
        assert_eq!(estimate(&lib, &ai(0.0, 0.0), &lang.id, Scope::Document).unwrap().images.calls, 0);
        assert_eq!(lib.reanalyze_other_model(&lang.id, "model-b").unwrap(), 1);
        assert_eq!(lib.get_item(&a.id).unwrap().analysis_status, "pending");
        assert_eq!(lib.get_item(&b.id).unwrap().analysis_status, "done");
        assert_eq!(lib.other_model_analyses(&lang.id, "model-a").unwrap().images, 1);
    }

    #[test]
    fn passes_on_the_chatgpt_plan_are_marked_and_have_no_price() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path()).unwrap();
        let lang = lib.create_language("Organic", "").unwrap();
        lib.import_bytes(&lang.id, "a.png", &png(3), None).unwrap();

        let mut plan = ProviderProfile::from_preset("chatgpt");
        plan.model = "gpt-x".into();
        // Prices from an earlier setting do not count: the plan pays.
        plan.price_in = 1.0;
        plan.price_out = 4.0;
        let both = AiConfig { analysis_profile: plan.id.clone(), synthesis_profile: plan.id.clone(), profiles: vec![plan.clone()] };
        let e = estimate(&lib, &both, &lang.id, Scope::Queue).unwrap();
        assert!(e.plan);
        assert_eq!(e.cost_usd, None);

        // The plan analyzes the images only. An estimate of the document alone does not use it.
        let mut keyed = ai(1.0, 4.0);
        keyed.analysis_profile = plan.id.clone();
        keyed.profiles.push(plan);
        let e = estimate(&lib, &keyed, &lang.id, Scope::Document).unwrap();
        assert_eq!((e.images.calls, e.document.calls), (0, 1));
        assert!(!e.plan, "no pass of this estimate uses the plan");
        assert!(e.cost_usd.is_some());
        // With the image pass in it, the plan is in use and the money is not known.
        let e = estimate(&lib, &keyed, &lang.id, Scope::Queue).unwrap();
        assert!(e.plan);
        assert_eq!(e.cost_usd, None);
        assert!(!estimate(&lib, &ai(0.0, 0.0), &lang.id, Scope::All).unwrap().plan);
    }
}
