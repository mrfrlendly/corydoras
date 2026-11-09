//! Prompts, image preparation and output checks for the two analysis passes.

use crate::models::*;
use crate::util::fnv1a64;
use anyhow::{anyhow, bail, Result};
use base64::Engine;
use image::{DynamicImage, ImageDecoder, ImageReader};
use serde_json::{json, Value};
use std::io::Cursor;
use std::path::Path;

/// Most items that one synthesis pass reads.
pub const SYNTH_ITEM_LIMIT: usize = 150;
/// Longest side of the full image that pass 1 sends (pixels).
const FULL_MAX_SIDE: u32 = 2048;
/// Longest side of a region crop that pass 1 sends (pixels).
const CROP_MAX_SIDE: u32 = 1300;

// ----------------------------------------------------------------------
// Images
// ----------------------------------------------------------------------

/// Decodes an image and applies its EXIF orientation.
pub fn decode_oriented(bytes: &[u8]) -> Result<DynamicImage> {
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder)?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok(img)
}

/// Scales the image so that its longest side is at most `max_side`, then encodes JPEG.
pub fn encode_jpeg(img: &DynamicImage, max_side: u32, quality: u8) -> Result<Vec<u8>> {
    let scaled = if img.width().max(img.height()) > max_side {
        img.resize(max_side, max_side, image::imageops::FilterType::Lanczos3)
    } else {
        img.clone()
    };
    let rgb = DynamicImage::ImageRgb8(scaled.to_rgb8());
    let mut buf = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
    rgb.write_with_encoder(enc)?;
    Ok(buf)
}

pub fn data_url_jpeg(bytes: &[u8]) -> String {
    format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// Decodes base64 (standard alphabet; whitespace and a data-URL prefix are ignored).
pub fn base64_decode(text: &str) -> anyhow::Result<Vec<u8>> {
    let body = text.rsplit_once(',').map(|(_, b)| b).unwrap_or(text);
    let clean: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(clean.as_bytes())
        .map_err(|e| anyhow::anyhow!("The image data is not valid base64: {e}"))
}

pub fn base64_of(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Reads a file and returns a JPEG with the longest side at most `max_side`.
pub fn jpeg_from_file(path: &Path, max_side: u32) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    let img = decode_oriented(&bytes)?;
    encode_jpeg(&img, max_side, 85)
}

/// The full image plus one crop for each region mark, as data URLs.
/// The crops are also saved in the "crops" folder of the language.
pub fn prepare_item_images(
    original: &Path,
    crops_dir: &Path,
    item_id: &str,
    regions: &[Region],
) -> Result<Vec<String>> {
    let bytes = std::fs::read(original)?;
    let img = decode_oriented(&bytes)?;
    let mut out = vec![data_url_jpeg(&encode_jpeg(&img, FULL_MAX_SIDE, 85)?)];
    if !regions.is_empty() {
        std::fs::create_dir_all(crops_dir)?;
    }
    let (w, h) = (img.width() as f64, img.height() as f64);
    for (i, r) in regions.iter().enumerate() {
        let x = (r.x * w).floor().max(0.0) as u32;
        let y = (r.y * h).floor().max(0.0) as u32;
        let cw = ((r.w * w).ceil() as u32).min(img.width().saturating_sub(x)).max(1);
        let ch = ((r.h * h).ceil() as u32).min(img.height().saturating_sub(y)).max(1);
        if cw < 8 || ch < 8 {
            continue;
        }
        let crop = img.crop_imm(x, y, cw, ch);
        let jpeg = encode_jpeg(&crop, CROP_MAX_SIDE, 90)?;
        let _ = std::fs::write(crops_dir.join(format!("{item_id}_r{}.jpg", i + 1)), &jpeg);
        out.push(data_url_jpeg(&jpeg));
    }
    Ok(out)
}

// ----------------------------------------------------------------------
// Pass 1: item analysis
// ----------------------------------------------------------------------

/// Changes to the prompts or the schema change this value. It is part of the
/// input fingerprint, so an upgrade makes the next synthesis run again.
pub const PIPELINE_VERSION: &str = "3";
/// Most sources that one synthesis pass reads.
pub const SYNTH_SOURCE_LIMIT: usize = 30;
/// Most characters of a source text that pass 1 sends.
pub const SOURCE_TEXT_LIMIT: usize = 60_000;

pub const ITEM_SYSTEM: &str = r#"You are a senior industrial designer. You study one reference image that belongs to a visual design language. Other agents will use your notes to model parts in CAD, to choose colours, materials and finishes, and to write prompts for concept images.

Rules:
- Describe what you see. Do not guess what you cannot see.
- Name the object family: the kind of object or element (bench, bracket, louvre facade, chair, lamp, enclosure). Then list its design features in short noun phrases (for example "laminated shell", "blade field", "branching bracket").
- Measurements: give a value only when you can see it. Each measurement must say exactly what it measures ("of"), its unit, and whether you measured it from the image or estimated it. Prefer ratios and angles. Use millimetres only when an object of known size in the image sets the scale.
- Do not give manufacturing values (wall thickness, tolerances, draft angles, clearances). An image cannot show the process. If the image shows something about a manufacturing method, describe it in words in "process_cues".
- Colours: separate object colours (the product: body, accent, detail, structure) from presentation colours (background, light, shadow, studio surface). A background is never an object colour.
- The designer can mark regions of the image. Each region comes as an extra image after the full image. Give each region its own entry in "details".
- The designer can mark an image as an ANTI-REFERENCE ("not this"). Then describe what the image shows that the language must AVOID.
- Write in clear, simple technical English: short sentences, active voice.
- Return only one JSON object. No markdown, no commentary."#;

pub fn item_output_shape() -> Value {
    json!({
        "summary": "Two or three sentences: what this image contributes to the language.",
        "role": "reference | anti-reference",
        "subject": "What the object or scene is.",
        "object_family": "The kind of object or element, for example bench, bracket, louvre facade.",
        "features": ["laminated shell", "blade field"],
        "form": {
            "primary_shapes": ["..."],
            "massing": "How the volumes are composed.",
            "silhouette": "Outline character.",
            "symmetry": "..."
        },
        "edges": {
            "treatment": "fillet / chamfer / sharp / mixed, and where",
            "transitions": "How surfaces meet."
        },
        "measurements": [
            {"name": "edge fillet radius", "value": 0.4, "unit": "ratio | deg | count | percent | mm", "of": "Exactly what is measured, for example: radius divided by the plate thickness at the free edge.", "basis": "measured | estimated", "confidence": 0.5}
        ],
        "surfaces": {
            "finish": "matte / satin / gloss / textured",
            "texture": "...",
            "parting_lines_and_seams": "...",
            "fasteners": "visible / hidden / none seen"
        },
        "materials": ["..."],
        "object_colours": [{"role": "body | accent | detail | structure", "hex": "#RRGGBB", "name": "...", "coverage": "Share of the object."}],
        "presentation_colours": [{"role": "background | light | shadow | surface | other", "hex": "#RRGGBB", "note": "..."}],
        "process_cues": "What the image shows about how the object is made, in words, or \"none seen\".",
        "details": [{"region": "r1", "tag": "...", "observation": "..."}],
        "graphics_and_type": "Labels, markings, typography, or none.",
        "light_and_presentation": "Light, background and camera (for render prompts).",
        "rules_suggested": {"do": ["..."], "dont": ["..."]},
        "keywords": ["..."],
        "confidence": 0.0
    })
}

pub fn item_user_text(language: &Language, item: &Item, regions: &[Region]) -> String {
    let mut s = String::new();
    s.push_str(&format!("Design language: {}\n", language.name));
    if !language.description.trim().is_empty() {
        s.push_str(&format!("Language description: {}\n", language.description.trim()));
    }
    s.push_str(&format!("Item ID: {}\n", item.id));
    s.push_str(&format!(
        "Role: {}\n",
        if item.anti {
            "ANTI-REFERENCE (\"not this\"): describe what the language must avoid"
        } else {
            "reference"
        }
    ));
    s.push_str(&format!("Designer weight: {}/5\n", item.weight));
    if !item.tags.is_empty() {
        s.push_str(&format!("Designer tags: {}\n", item.tags.join(", ")));
    }
    if !item.note.trim().is_empty() {
        s.push_str(&format!("Designer note: {}\n", item.note.trim()));
    }
    s.push_str(&format!("Image size: {} x {} px\n", item.width, item.height));
    s.push_str("\nImage 1 is the full reference.\n");
    for (i, r) in regions.iter().enumerate() {
        s.push_str(&format!(
            "Image {} is region r{} (x {:.0}%, y {:.0}%, w {:.0}%, h {:.0}% of the full image){}{}\n",
            i + 2,
            i + 1,
            r.x * 100.0,
            r.y * 100.0,
            r.w * 100.0,
            r.h * 100.0,
            if r.tag.is_empty() { String::new() } else { format!(", tag \"{}\"", r.tag) },
            if r.note.is_empty() { String::new() } else { format!(", note: {}", r.note) },
        ));
    }
    s.push_str("\nReturn a json object with this shape (replace the example values):\n");
    s.push_str(&serde_json::to_string_pretty(&item_output_shape()).unwrap_or_default());
    s
}

// ----------------------------------------------------------------------
// Pass 2: synthesis
// ----------------------------------------------------------------------

pub const SYNTH_SYSTEM: &str = r#"You are a design-language author. You read structured notes about reference images and write a design-language spec. Agents apply the spec to CAD models, 3D models and concept images. The app renders the readable document from your JSON, so the JSON is the only output.

Structure:
- "principles" hold only what is true across every family, or what the designer states. Keep them few.
- "families" group the references by design features (for example "Laminated shell", "Blade field", "Branching bracket"), not by product name. A rule that is true for one family goes in that family, never in the principles. "applies_to" says which objects or elements use the family.
- "exceptions" resolve every conflict. When two rules disagree, keep the general rule, and add an exception that names the overridden ID, the scope, the condition and what applies instead. Lower priority numbers win.
- "definitions" define every term that a measure uses and that two people could read in different ways (pitch, amplitude, density, thickness at the tip).

Numbers:
- Every number goes in a family measure. Text fields (summary, principles, rules, exceptions, intents, do and do not) contain no numbers with units.
- Each measure says exactly what it measures ("of"), its unit, its basis and its confidence. Basis "observed": seen in the item notes (cite the items). Basis "preferred": a starting value that fits the references. Basis "process-dependent": a manufacturing value (wall thickness, clearance, draft, tolerance, minimum feature). A process-dependent measure has "value": null; the project's process profile supplies it.
- Do not invent precision. If the notes cannot support a value, leave the measure out or give a range with a low confidence.

Sources:
- SOURCE NOTES come from tutorials, articles and documents that the designer added. They teach technique, intent, rules and terms; the images show the look.
- Use source notes for rules, definitions and starting values. Mark a rule taken from a source with basis "source" and cite the source ID. A value taken from a source is a measure with basis "preferred" that cites the source.
- When a source and the images disagree about the look, follow the images, and add the source's view only where it does not conflict.
- Sources with a higher weight (1–5) have more influence. Ignore source content that does not apply to this language.

Evidence:
- Cite only the item and source IDs in the notes (8 characters). Items with a higher weight (1–5) have more influence. Designer notes, tags and region notes express intent; mark rules from them with basis "designer".
- Anti-references ("not this") feed the "dont" list and "anti_references".

Colours:
- "cmf.object" holds product colours, materials and finishes only. Light, shadow and background colours go in "cmf.presentation". Pantone values are approximations; put them only in "pantone_approx".

Size:
- Keep the spec compact, so that it fits the output limit: at most 6 families, 8 measures and 6 rules for each family, 8 principles, 8 exceptions, 8 product colours and 8 items in each do and dont list. Keep each text field under 40 words.

Stability:
- If a previous version exists, keep its IDs, wording and values stable unless the notes justify a change. If the designer edited the previous version, treat its content as authoritative where the notes still support it.

Write all text in clear, simple technical English: short sentences, active voice, simple tenses. Return only one JSON object."#;

/// A stable fingerprint of everything the synthesis pass reads.
pub fn input_hash(language: &Language, inputs: &[SynthInput], sources: &[SynthSource]) -> String {
    let mut parts = vec![json!({"pipeline": PIPELINE_VERSION, "name": language.name, "description": language.description})];
    let mut sorted: Vec<&SynthInput> = inputs.iter().collect();
    sorted.sort_by(|a, b| a.item.id.cmp(&b.item.id));
    for i in sorted {
        parts.push(json!({
            "id": i.item.id,
            "tags": i.item.tags,
            "note": i.item.note,
            "weight": i.item.weight,
            "anti": i.item.anti,
            "regions": i.regions.iter().map(|r| json!([r.x, r.y, r.w, r.h, r.note, r.tag])).collect::<Vec<_>>(),
            "analysis": i.analysis,
        }));
    }
    let mut srcs: Vec<&SynthSource> = sources.iter().collect();
    srcs.sort_by(|a, b| a.source.id.cmp(&b.source.id));
    for x in srcs {
        parts.push(json!({"source": x.source.id, "note": x.source.note, "weight": x.source.weight, "title": x.source.title, "analysis": x.analysis}));
    }
    fnv1a64(serde_json::to_string(&parts).unwrap_or_default().as_bytes())
}

pub fn synth_user_text(
    language: &Language,
    inputs: &[SynthInput],
    sources: &[SynthSource],
    previous: Option<&VersionContent>,
) -> String {
    let mut s = String::new();
    s.push_str(&format!("Design language: {} (ID {})\n", language.name, language.id));
    if !language.description.trim().is_empty() {
        s.push_str(&format!("Description from the designer: {}\n", language.description.trim()));
    }
    let anti = inputs.iter().filter(|i| i.item.anti).count();
    s.push_str(&format!(
        "Items: {} references and {} anti-references.\n",
        inputs.len() - anti,
        anti
    ));
    if !sources.is_empty() {
        s.push_str(&format!("Sources: {} tutorials or documents.\n", sources.len()));
    }
    let mut ids: Vec<String> = inputs.iter().map(|i| i.item.id.chars().take(8).collect()).collect();
    ids.extend(sources.iter().map(|x| x.source.id.chars().take(8).collect::<String>()));
    s.push_str(&format!("Valid evidence IDs: {}\n\n", ids.join(", ")));
    s.push_str("ITEM NOTES\n");
    for i in inputs {
        let short: String = i.item.id.chars().take(8).collect();
        let entry = json!({
            "id": short,
            "role": if i.item.anti { "anti-reference" } else { "reference" },
            "weight": i.item.weight,
            "designer_tags": i.item.tags,
            "designer_note": i.item.note,
            "designer_regions": i.regions.iter().enumerate().map(|(n, r)| json!({"region": format!("r{}", n + 1), "tag": r.tag, "note": r.note})).collect::<Vec<_>>(),
            "analysis": i.analysis,
        });
        s.push_str(&serde_json::to_string(&entry).unwrap_or_default());
        s.push('\n');
    }
    if !sources.is_empty() {
        s.push_str("\nSOURCE NOTES\n");
        for x in sources {
            let entry = json!({
                "id": x.source.id.chars().take(8).collect::<String>(),
                "title": x.source.title,
                "kind": x.source.media,
                "from": x.source.url.clone().unwrap_or_else(|| x.source.original_name.clone()),
                "weight": x.source.weight,
                "designer_note": x.source.note,
                "notes": x.analysis,
            });
            s.push_str(&serde_json::to_string(&entry).unwrap_or_default());
            s.push('\n');
        }
    }
    if let Some(prev) = previous {
        s.push_str(&format!(
            "\nPREVIOUS VERSION v{}{}\n",
            prev.info.version,
            if prev.info.edited { " (edited by the designer)" } else { "" }
        ));
        if prev.info.schema >= 2 {
            s.push_str("spec:\n");
            s.push_str(&serde_json::to_string(&prev.tokens).unwrap_or_default());
        } else {
            s.push_str("(Older format. Reuse its intent, but restructure it into the new spec.)\nlanguage_md:\n");
            s.push_str(&prev.markdown);
            s.push_str("\ntokens:\n");
            s.push_str(&serde_json::to_string(&prev.tokens).unwrap_or_default());
        }
        s.push('\n');
    }
    s.push_str("\nReturn a json object with this shape (replace the example values):\n");
    s.push_str(&serde_json::to_string_pretty(&crate::spec::output_shape()).unwrap_or_default());
    s
}

// ----------------------------------------------------------------------
// Output checks
// ----------------------------------------------------------------------

/// Parses a JSON object from model output. Accepts code fences and text around the object.
pub fn parse_json_loose(text: &str) -> Result<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        if v.is_object() {
            return Ok(v);
        }
    }
    let start = t.find('{').ok_or_else(|| anyhow!("The model output has no JSON object."))?;
    let end = t.rfind('}').ok_or_else(|| anyhow!("The model output has no JSON object."))?;
    if end <= start {
        bail!("The model output has no complete JSON object.");
    }
    let v: Value = serde_json::from_str(&t[start..=end])
        .map_err(|e| anyhow!("The model output is not valid JSON: {e}"))?;
    if !v.is_object() {
        bail!("The model output is not a JSON object.");
    }
    Ok(v)
}

/// Checks the structure of tokens.json.
pub fn validate_tokens(t: &Value) -> Result<()> {
    let obj = t.as_object().ok_or_else(|| anyhow!("tokens must be an object"))?;
    if !obj.get("geometry").map(|v| v.is_object()).unwrap_or(false) {
        bail!("\"geometry\" must be an object");
    }
    let cmf = obj
        .get("cmf")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("\"cmf\" must be an array"))?;
    for (i, c) in cmf.iter().enumerate() {
        if !c.is_object() {
            bail!("\"cmf[{i}]\" must be an object");
        }
        if let Some(hex) = c.get("hex").and_then(|h| h.as_str()) {
            let ok = hex.len() == 7
                && hex.starts_with('#')
                && hex[1..].chars().all(|ch| ch.is_ascii_hexdigit());
            if !ok {
                bail!("\"cmf[{i}].hex\" must look like #RRGGBB (found \"{hex}\")");
            }
        }
    }
    let prompts = obj
        .get("prompts")
        .and_then(|v| v.as_object())
        .ok_or_else(|| anyhow!("\"prompts\" must be an object"))?;
    for k in ["positive", "negative"] {
        if !prompts.get(k).map(|v| v.is_string()).unwrap_or(false) {
            bail!("\"prompts.{k}\" must be a string");
        }
    }
    for k in ["do", "dont"] {
        if !obj.get(k).map(|v| v.is_array()).unwrap_or(false) {
            bail!("\"{k}\" must be an array");
        }
    }
    Ok(())
}

/// Checks the full synthesis output and returns (markdown, tokens).
pub fn check_synth_output(v: &Value) -> Result<(String, Value)> {
    let md = v
        .get("language_md")
        .and_then(|m| m.as_str())
        .filter(|m| m.trim().len() > 40)
        .ok_or_else(|| anyhow!("\"language_md\" must be a markdown string"))?
        .to_string();
    let tokens = v.get("tokens").cloned().ok_or_else(|| anyhow!("\"tokens\" is missing"))?;
    validate_tokens(&tokens)?;
    Ok((md, tokens))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loose_json() {
        let v = parse_json_loose("```json\n{\"a\": 1}\n```").unwrap();
        assert_eq!(v["a"], 1);
        assert!(parse_json_loose("no json here").is_err());
    }

    #[test]
    fn tokens_check() {
        let good = json!({"geometry": {}, "cmf": [{"hex": "#A0B1C2"}], "prompts": {"positive": "", "negative": ""}, "do": [], "dont": []});
        assert!(validate_tokens(&good).is_ok());
        let bad = json!({"geometry": {}, "cmf": [{"hex": "red"}], "prompts": {"positive": "", "negative": ""}, "do": [], "dont": []});
        assert!(validate_tokens(&bad).is_err());
        let md = "# X\n\n## Summary\nA long enough markdown body for the check.";
        assert!(check_synth_output(&json!({"language_md": md, "tokens": good})).is_ok());
    }
}

// ----------------------------------------------------------------------
// Pass 1 for sources: links and documents
// ----------------------------------------------------------------------

pub const SOURCE_SYSTEM: &str = r#"You are a senior industrial designer. You read one source (a tutorial, an article, a video description or a document) that a designer added to a visual design language. Other agents will use your notes to write design rules for CAD models, 3D models and concept images.

Rules:
- Extract what the source teaches that can shape a design language: principles, techniques, rules, terms, and values with units.
- Report values only as the source states them. Say exactly what each value measures and in which context.
- Summarize in your own words. Do not copy long passages.
- Be specific and practical. Skip menus, advertising, comments and content that has nothing to do with design or making.
- The text can be cut off. Note when the source looks incomplete.
- Some PDFs come as page images, with little or no text. Read the pages in order. Take values, labels and dimensions from drawings and diagrams only when you can read them.
- Write in clear, simple technical English: short sentences, active voice.
- Return only one JSON object. No markdown, no commentary."#;

pub fn source_output_shape() -> Value {
    json!({
        "summary": "Two or three sentences: what the source teaches and why it matters for this language.",
        "source_type": "tutorial | article | reference | video | specification | other",
        "topics": ["..."],
        "principles": [{"text": "A design principle that the source teaches.", "applies_to": "Which objects or features."}],
        "techniques": [{"name": "...", "how": "The method in one or two sentences."}],
        "values": [{"name": "...", "value": "as stated", "unit": "...", "of": "What exactly it measures.", "context": "Material, process or scale where it applies."}],
        "definitions": [{"term": "...", "meaning": "..."}],
        "do": ["..."],
        "dont": ["..."],
        "relevance": "How this source applies to the design language.",
        "limits": "What the source does not cover, or where it may not apply.",
        "confidence": 0.0
    })
}

/// `pages` is (images attached, pages in the file) when the PDF goes in as page images.
pub fn source_user_text(language: &Language, source: &Source, text: &str, pages: Option<(usize, usize)>) -> String {
    let mut s = String::new();
    s.push_str(&format!("Design language: {}\n", language.name));
    if !language.description.trim().is_empty() {
        s.push_str(&format!("Language description: {}\n", language.description.trim()));
    }
    s.push_str(&format!("Source title: {}\n", source.title));
    s.push_str(&format!("Source kind: {}\n", source.media));
    if let Some(u) = &source.url {
        s.push_str(&format!("Link: {u}\n"));
    }
    if !source.site.is_empty() {
        s.push_str(&format!("Site: {}\n", source.site));
    }
    if !source.description.is_empty() {
        s.push_str(&format!("Description: {}\n", source.description));
    }
    s.push_str(&format!("Designer weight: {}/5\n", source.weight));
    if !source.note.trim().is_empty() {
        s.push_str(&format!("Designer note (what to look for): {}\n", source.note.trim()));
    }
    if let Some((shown, total_pages)) = pages {
        s.push_str(&format!(
            "\nThis PDF has {total_pages} page{}. {} follow{} as images, in page order.{}\n",
            if total_pages == 1 { "" } else { "s" },
            if shown == total_pages { format!("All {shown} pages") } else { format!("The first {shown} pages") },
            if shown == 1 { "s" } else { "" },
            if text.trim().is_empty() { " The file has no text layer." } else { " The text layer is below; the images show drawings, figures and layout." },
        ));
    }
    let total = text.chars().count();
    let body: String = text.chars().take(SOURCE_TEXT_LIMIT).collect();
    if text.trim().is_empty() {
        s.push_str("\nSOURCE TEXT: none. Use the page images.\n");
    } else {
    s.push_str(&format!(
        "\nSOURCE TEXT ({} characters{}):\n<<<\n{}\n>>>\n",
        total,
        if total > SOURCE_TEXT_LIMIT { format!(", the first {SOURCE_TEXT_LIMIT} shown") } else { String::new() },
        body
    ));
    }
    if source.media == "video" {
        s.push_str("\nThis is a video. Only its title and description are available, not the spoken content. Keep the notes short, and set a low confidence.\n");
    }
    s.push_str("\nReturn a json object with this shape (replace the example values):\n");
    s.push_str(&serde_json::to_string_pretty(&source_output_shape()).unwrap_or_default());
    s
}
