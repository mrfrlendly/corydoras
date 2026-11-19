# Midjourney Prompt Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Each new language version holds a Midjourney prompt for the whole language and for each family, shown in the document as a line to paste with an object slot.

**Architecture:** The synthesis pass writes prompt text into a new optional key, `prompts.midjourney`, of tokens.json (schema 2, no schema bump). `spec.rs` repairs, validates and renders it: the app adds the slot `[your object]` and the parameters, so the syntax is always valid. The document view adds a Copy button to fenced blocks.

**Tech Stack:** Rust (`corydoras-core`, `corydoras-mcp`, serde_json), React 19 + TypeScript (react-markdown), Tauri 2.

**Spec:** `docs/superpowers/specs/2026-11-18-midjourney-prompt-design.md`

## Global Constraints

- Work on branch `midjourney-prompt`, cut from `main`. Commit only when the designer says so; each task names the commit it would be.
- `~/Corydoras` is the designer's live library. Never write to it. The live check uses a copy.
- The key `prompts.midjourney` is optional everywhere. No `SCHEMA_VERSION` change. The fixture `spec::tests::sample()` stays unchanged: it is the proof that older versions still pass.
- The assembled line is exactly `[your object], <text> --no <no> --raw`. An empty `no` leaves `--no` out. No other parameter.
- Model instruction limits: at most 40 words for each text, at most 8 items in `no`. The validator warns above 60 words.
- All user-facing text follows the repo's style: short sentences, active voice, simple tenses.

## Review Focus

1. **A version written before this change** (no `midjourney` key): the document has no Midjourney section, no warning, and a hand edit saves. Test in Task 1 (`midjourney_is_optional`) and Task 2 (`render_midjourney`).
2. **The model writes the slot or parameters itself** (`/imagine prompt: [object], … --ar 16:9 --v 7`): the stored text has neither, and the line has one slot and only `--raw`. Test in Task 1 (`midjourney_repair`).
3. **A family has no prompt, or the prompt names a family that does not exist:** the family brief shows the language prompt; an unknown key is not rendered; a warning names it. Tests in Task 1 (`midjourney_soft_warnings`) and Task 2 (`render_midjourney`).
4. **Text that breaks the fenced block or the line** (a newline, backticks, from a hand edit): the rendered line is one line with no backtick. Test in Task 2 (`midjourney_line_format`).
5. **An empty exclusion list:** no dangling `--no`. Test in Task 2 (`midjourney_line_format`).

---

### Task 1: Data, repair and validation

**Files:**
- Modify: `crates/corydoras-core/src/spec.rs` (`output_shape` ~54, `repair` ~333, `validate` ~868, tests ~1415)
- Modify: `crates/corydoras-core/src/analysis.rs` (`PIPELINE_VERSION` 113, `SYNTH_SYSTEM` 221)

**Interfaces:**
- Produces: `pub const MIDJOURNEY_SLOT: &str = "[your object]"`, `pub const MIDJOURNEY_PARAMS: &str = "--raw"`, `pub const MIDJOURNEY_WORD_WARN: usize = 60`, `fn clean_midjourney(text: &str) -> String` (collapses all whitespace to single spaces, removes backticks, trims), and the test fixture `pub fn sample_mj() -> Value` in `spec::tests`.

- [ ] **Step 1: Add the fixture and the failing tests** in `spec::tests`

```rust
pub fn sample_mj() -> Value {
    let mut t = sample();
    t["prompts"]["midjourney"] = json!({
        "language": "product concept render, one continuous matte shell, soft tangent surfaces, warm white oak veneer, soft studio light, pale neutral background",
        "no": "chrome, visible frame",
        "families": {
            "shell": "product concept render, single soft laminated shell, seams along the flow lines, warm white oak veneer, matte finish, soft studio light",
            "blade-field": "product concept render, field of thin parallel blades at a constant spacing, warm white matte finish, soft studio light"
        }
    });
    t
}

#[test]
fn midjourney_is_optional() {
    let c = validate(&sample_mj(), &known());
    assert!(c.errors.is_empty(), "{:?}", c.errors);
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    let mut t = sample();
    t["prompts"]["midjourney"] = Value::Null;
    let c = validate(&t, &known());
    assert!(c.errors.is_empty() && c.warnings.is_empty(), "{c:?}");
}

#[test]
fn midjourney_hard_errors() {
    let bad = |v: Value| { let mut t = sample_mj(); t["prompts"]["midjourney"] = v; validate(&t, &known()).errors.join(" ") };
    assert!(bad(json!("text")).contains("\"prompts.midjourney\" must be an object"));
    assert!(bad(json!({"language": 5})).contains("\"prompts.midjourney.language\" must be a string"));
    assert!(bad(json!({"no": "x"})).contains("\"prompts.midjourney.language\" must be a string"));
    assert!(bad(json!({"language": "a", "no": ["x"]})).contains("\"prompts.midjourney.no\" must be a string"));
    assert!(bad(json!({"language": "a", "families": {"shell": 3}})).contains("\"prompts.midjourney.families\" must be an object of strings"));
}

#[test]
fn midjourney_soft_warnings() {
    let warn = |f: &dyn Fn(&mut Value)| { let mut t = sample_mj(); f(&mut t["prompts"]["midjourney"]); let c = validate(&t, &known()); assert!(c.errors.is_empty(), "{:?}", c.errors); c.warnings.join(" | ") };
    assert!(warn(&|m| m["families"]["nope"] = json!("x")).contains("prompts.midjourney.families has \"nope\", which is not a family ID."));
    assert!(warn(&|m| { m["families"].as_object_mut().unwrap().remove("shell"); }).contains("No Midjourney prompt for family shell."));
    assert!(warn(&|m| m["language"] = json!("body colour #E8E4DC, matte")).contains("prompts.midjourney.language has a hex code"));
    assert!(warn(&|m| m["families"]["shell"] = json!("soft shell --ar 16:9")).contains("prompts.midjourney.families.shell has a parameter"));
    assert!(warn(&|m| m["language"] = json!(vec!["word"; 61].join(" "))).contains("prompts.midjourney.language has 61 words"));
    assert!(warn(&|m| m["language"] = json!(" ")).contains("prompts.midjourney.language is empty"));
}

#[test]
fn midjourney_repair() {
    let mut t = sample_mj();
    t["prompts"]["midjourney"]["language"] = json!("/imagine prompt: [object], matte shell, soft light --ar 16:9 --v 7.");
    t["prompts"]["midjourney"]["no"] = json!("--no chrome, frame --raw");
    t["prompts"]["midjourney"]["families"]["shell"] = json!(["soft\nshell", "with `seams`"]);
    let rep = repair(&mut t);
    let m = &t["prompts"]["midjourney"];
    assert_eq!(m["language"], "matte shell, soft light");
    assert_eq!(m["no"], "chrome, frame");
    assert_eq!(m["families"]["shell"], "soft shell, with seams");
    assert!(rep.fixed.iter().any(|f| f.contains("prompts.midjourney")));
    let c = validate(&t, &known());
    assert!(c.errors.is_empty() && c.warnings.is_empty(), "{c:?}");

    let mut t = sample_mj();
    t["prompts"]["midjourney"] = json!("matte shell");
    repair(&mut t);
    assert_eq!(t["prompts"]["midjourney"]["language"], "matte shell");

    let mut t = sample();
    let rep = repair(&mut t);
    assert!(rep.flagged.iter().any(|f| f == "The model gave no Midjourney prompt."));
    assert!(t["prompts"].get("midjourney").is_none());
}
```

Also add to `analysis.rs` tests:

```rust
#[test]
fn synthesis_asks_for_midjourney() {
    assert!(crate::spec::output_shape()["prompts"]["midjourney"]["language"].is_string());
    assert!(SYNTH_SYSTEM.contains("\"prompts.midjourney\""));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p corydoras-core midjourney`
Expected: FAIL (the fixture compiles; the assertions fail, `sample_mj` validates with no check yet for the bad cases).

- [ ] **Step 3: Implement**

- `output_shape`: under `prompts` add `"midjourney": {"language": "Short phrases for the look that every family shares. No object name, no parameters.", "no": "Comma-separated nouns to leave out.", "families": {"blade-field": "A complete description for this family only."}}`.
- `repair`, inside the `Some(Value::Object(p))` arm of the prompts block: an absent or null key pushes the flag `The model gave no Midjourney prompt.` and stores nothing. A string becomes `{"language": <string>}`. In `language`, `no` and each family text: a list joins with `", "`; then `clean_midjourney`; then remove a leading `/imagine`, `prompt:` and a leading `[…]` slot with its comma; cut the text at the first `--` that starts the text or follows a space (for `no`, first drop a leading `--no`); trim trailing `, . ;`. Any change beyond whitespace pushes one `fixed` line that names the field. A missing `no` becomes `""`. A `families` value that is not an object is removed.
- `validate`, inside the arm that accepted `positive` and `negative`: errors and warnings exactly as the tests word them. The word-count warning reads `{where} has {n} words. Midjourney works best with at most 40.` and fires above `MIDJOURNEY_WORD_WARN`. A hex code is `#` plus 3 or 6 hex digits. The missing-family warning is one line per family: `No Midjourney prompt for family {id}.`
- `analysis.rs`: `PIPELINE_VERSION` becomes `"4"`. Add this block to `SYNTH_SYSTEM` between `Colours:` and `Size:`:

```text
Midjourney:
- "prompts.midjourney" holds prompt text for the Midjourney image tool. The app adds the object at the start and the parameters at the end. Do not write an object name, a slot or "--" parameters.
- "language" describes the look that every family shares. Each entry in "families" (one for each family ID) is complete: it must work without "language".
- Write a few short descriptive phrases in this order: medium (for example "product concept render"), form and surfaces, material and finish, colour, light and background. Use at most 40 words. Do not write long keyword lists.
- Use words that an outsider understands. Name colours in words, not hex codes. Give no dimensions. Do not use the language name, a family name or a term from "definitions".
- Take the content from this spec: product colours and finishes from "cmf.object", light and background from "cmf.presentation".
- Say what the image shows. Put what to leave out in "no": at most 8 nouns for visible things, separated by commas (for example "chrome, visible screws"). Midjourney reads each word of "no" alone, so do not use a word that is wrong on its own.
```

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p corydoras-core`
Expected: PASS, including the unchanged `valid_sample`, `repair_fixes_small_problems` and `repair_schema1_cmf_and_missing_parts`.

- [ ] **Step 5: Commit (if asked)** `feat: store a Midjourney prompt in the language spec`

---

### Task 2: Render the section

**Files:**
- Modify: `crates/corydoras-core/src/spec.rs` (after `sec_prompts` ~1259; `render_markdown`, `render_family_brief`, `render_scope`)

**Interfaces:**
- Consumes: `MIDJOURNEY_SLOT`, `MIDJOURNEY_PARAMS`, `clean_midjourney`, `sample_mj` (Task 1).
- Produces: `pub fn midjourney_line(text: &str, no: &str) -> String` and `fn sec_midjourney(t: &Value, out: &mut String, family: Option<&str>)`.

- [ ] **Step 1: Write the failing tests** in `spec::tests`

```rust
#[test]
fn midjourney_line_format() {
    assert_eq!(midjourney_line("matte shell", "chrome, frame"), "[your object], matte shell --no chrome, frame --raw");
    assert_eq!(midjourney_line("matte shell", " "), "[your object], matte shell --raw");
    assert_eq!(midjourney_line("soft\nshell  `x`", ""), "[your object], soft shell x --raw");
}

#[test]
fn render_midjourney() {
    let mut t = sample_mj();
    finalize(&mut t, "organic", 3);
    let md = render_markdown("Organic", &t, &[]);
    let at = |s: &str| md.find(s).unwrap_or_else(|| panic!("missing {s}"));
    assert!(at("## Image-generation tokens") < at("## Midjourney prompt") && at("## Midjourney prompt") < at("## Do and do not"));
    assert!(md.contains("Replace `[your object]` with the thing to draw. Keep the rest of the line as it is."));
    assert!(md.contains("**Whole language**\n\n```text\n[your object], product concept render, one continuous matte shell, soft tangent surfaces, warm white oak veneer, soft studio light, pale neutral background --no chrome, visible frame --raw\n```"));
    assert!(at("**Laminated shell** (`shell`)") < at("**Blade field** (`blade-field`)")); // order of the families array

    let fam = render_family_brief("Organic", &t, "blade-field").unwrap();
    assert!(fam.contains("field of thin parallel blades"));
    assert!(!fam.contains("single soft laminated shell") && !fam.contains("one continuous matte shell"));

    let mut gap = t.clone();
    gap["prompts"]["midjourney"]["families"].as_object_mut().unwrap().remove("blade-field");
    gap["prompts"]["midjourney"]["families"]["nope"] = json!("never shown");
    let fam = render_family_brief("Organic", &gap, "blade-field").unwrap();
    assert!(fam.contains("**Whole language**") && fam.contains("one continuous matte shell"));
    assert!(!render_markdown("Organic", &gap, &[]).contains("never shown"));

    assert!(render_scope("Organic", &t, Scope::Prompts).contains("## Midjourney prompt"));
    assert!(!render_scope("Organic", &t, Scope::Cad).contains("Midjourney"));
    assert!(!render_markdown("Organic", &sample(), &[]).contains("Midjourney"));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p corydoras-core midjourney`
Expected: FAIL to compile, `midjourney_line` not found.

- [ ] **Step 3: Implement** `midjourney_line` and `sec_midjourney`; call `sec_midjourney` right after `sec_prompts` in `render_markdown`, `render_family_brief` and the `Scope::Prompts` arm.

The section is left out when the key is absent, or when it has no non-empty text to show. Both the text and `no` go through `clean_midjourney`. With `family: Some(f)`: show that family's block only, or the "Whole language" block when the family has no text.

- [ ] **Step 4: Run to verify they pass**

Run: `cargo test -p corydoras-core`
Expected: PASS.

- [ ] **Step 5: Commit (if asked)** `feat: show the Midjourney prompt in the language document`

---

### Task 3: Version comparison and inheritance

**Files:**
- Modify: `crates/corydoras-core/src/diff.rs` (`semantic`, after the prompt families block ~266; tests ~368)
- Test only: `crates/corydoras-core/src/inherit.rs` (tests ~440)

**Interfaces:**
- Consumes: `spec::tests::sample_mj` (Task 1).

- [ ] **Step 1: Write the failing test** in `diff.rs` tests

```rust
#[test]
fn midjourney_changes_in_words() {
    let (a, b) = (crate::spec::tests::sample(), crate::spec::tests::sample_mj());
    assert!(semantic(&b, &b).is_empty());
    let c = semantic(&a, &b);
    let has = |c: &[Change], needle: &str| c.iter().any(|x| x.section == "Midjourney prompt" && x.text.contains(needle));
    assert!(has(&c, "The Midjourney prompt for the whole language is now: “product concept render, one continuous matte shell"), "{c:#?}");
    assert!(has(&c, "The Midjourney exclusions are now: “chrome, visible frame”"));
    assert!(has(&c, "Midjourney prompt for family shell added: “product concept render, single soft laminated shell"));
    let mut b2 = b.clone();
    b2["prompts"]["midjourney"]["families"]["blade-field"] = json!("thin blades");
    b2["prompts"]["midjourney"]["families"].as_object_mut().unwrap().remove("shell");
    let c = semantic(&b, &b2);
    assert!(has(&c, "The Midjourney prompt for family blade-field is now: “thin blades”"));
    assert!(has(&c, "Midjourney prompt for family shell removed."));
    assert!(has(&semantic(&b, &a), "The Midjourney prompt for the whole language was removed."));
}
```

And the inheritance check in `inherit.rs` tests (expected to pass without a code change; if it fails, fix `diff_into`):

```rust
#[test]
fn midjourney_prompt_patch() {
    let base = crate::spec::tests::sample_mj();
    let mut target = base.clone();
    target["prompts"]["midjourney"]["families"]["blade-field"] = json!("thin blades");
    let patch = diff_patch(&base, &target);
    assert_eq!(patch.len(), 1);
    assert_eq!(patch[0]["path"], "/prompts/midjourney/families/blade-field");
    let (out, _) = apply_patch(&base, &Value::Array(patch)).unwrap();
    assert_eq!(out, target);
    let patch = diff_patch(&sample(), &base);
    assert_eq!(patch[0]["op"], "add");
    assert_eq!(patch[0]["path"], "/prompts/midjourney");
}
```

- [ ] **Step 2: Run to verify** `cargo test -p corydoras-core midjourney` — `midjourney_changes_in_words` FAILS, `midjourney_prompt_patch` passes.

- [ ] **Step 3: Implement** the Midjourney block in `semantic` with the `push` helper: section `Midjourney prompt`; kinds `changed`, `added`, `removed`; subjects `language`, `no`, and the family ID. An empty new text reads `… was removed.`

- [ ] **Step 4: Run** `cargo test -p corydoras-core` — PASS.

- [ ] **Step 5: Commit (if asked)** `feat: list Midjourney prompt changes between versions`

---

### Task 4: Copy button in the document view

**Files:**
- Modify: `src/components/DocumentView.tsx` (`MARKDOWN` ~119)
- Modify: `src/styles.css` (after `.markdown code` ~1401)

- [ ] **Step 1: Implement** a `CodeBlock` component and register it as `pre` in `MARKDOWN`.

`CodeBlock({ node, children })` renders `<div className="code-block"><pre>{children}</pre><button className="btn small code-copy">…</button></div>`. The button copies `nodeText(node)` without the trailing newline through the existing `copyText`, shows the `Check` icon and "Copied" for 1.5 s, then returns to `Copy`. Title: `Copy this line`. CSS: the block is `position: relative`; the button sits top right; `.markdown pre` wraps long lines (`white-space: pre-wrap; overflow-wrap: anywhere`) and leaves room for the button; inline `code` styling must not add a second background inside `pre`.

- [ ] **Step 2: Verify** `npm run build` — exits 0 (type-check and bundle).

- [ ] **Step 3: Commit (if asked)** `feat: copy fenced lines from the language document`

---

### Task 5: Tell agents and the README

**Files:**
- Modify: `crates/corydoras-mcp/src/main.rs` (`INSTRUCTIONS` 26, `get_language` description ~81)
- Modify: `README.md` (spec table ~215)

- [ ] **Step 1: Edit the text**

- `INSTRUCTIONS`, one added sentence: `For concept images in Midjourney, use the "Midjourney prompt" section of the language document: replace [your object] with the object and keep the rest of the line.`
- `get_language` description, appended: ` The document has a "Midjourney prompt" section when the version has one: lines to paste, with a slot for the object.`
- README table row after `prompts`: `` | `prompts.midjourney` | Prompt text for Midjourney: one for the whole language, one for each family, and one exclusion list. The document shows each as a line to paste: `[your object], <text> --no <exclusions> --raw` | ``
- README, one paragraph under the table: a language gets the prompt with its next version; a language that extends a base carries the base's prompt.

- [ ] **Step 2: Verify** `cargo test -p corydoras-mcp` — PASS.

- [ ] **Step 3: Commit (if asked)** `docs: describe the Midjourney prompt for agents and in the README`

---

### Task 6: Whole check

- [ ] **Step 1:** `cargo test --workspace` — PASS.
- [ ] **Step 2:** `npm run build` — exits 0.
- [ ] **Step 3: Live check on a copy of the library** (read the `corydoras-live-check` memory first): save a hand edit of a real language's tokens.json that adds `prompts.midjourney`; confirm the "Midjourney prompt" section renders, the Copy button puts the exact line on the clipboard, an older version shows no section, and the Changes view lists the new prompt.
- [ ] **Step 4:** Report what a real synthesis run was not used for: the model instruction is covered by unit tests only until the designer updates a language.
